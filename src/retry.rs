use crate::errors::KodeBridgeError;
use rand::{random_range, rngs::StdRng, SeedableRng};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Type alias for complex retry function
pub type RetryFn = Box<dyn Fn(&KodeBridgeError, usize) -> bool + Send + Sync>;

/// Advanced retry configuration with adaptive strategies
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: usize,
    /// Base delay between retries
    pub base_delay: Duration,
    /// Maximum delay between retries (for exponential backoff)
    pub max_delay: Duration,
    /// Backoff strategy to use
    pub backoff_strategy: BackoffStrategy,
    /// Jitter strategy to avoid thundering herd
    pub jitter_strategy: JitterStrategy,
    /// Custom retry decision function (not cloneable, so we'll skip it in Clone)
    pub should_retry_fn: Option<RetryFn>,
}

impl Clone for RetryConfig {
    fn clone(&self) -> Self {
        Self {
            max_attempts: self.max_attempts,
            base_delay: self.base_delay,
            max_delay: self.max_delay,
            backoff_strategy: self.backoff_strategy,
            jitter_strategy: self.jitter_strategy,
            should_retry_fn: None, // Skip cloning function pointer
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed,
    /// Exponential backoff: delay *= multiplier
    Exponential { multiplier: f64 },
    /// Linear backoff: delay += increment
    Linear { increment: Duration },
}

#[derive(Debug, Clone, Copy)]
pub enum JitterStrategy {
    /// No jitter
    None,
    /// Add random jitter up to 50% of delay
    Full,
    /// Add random jitter up to 25% of delay  
    Partial,
    /// Use decorrelated jitter for better distribution
    Decorrelated,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_strategy: BackoffStrategy::Exponential { multiplier: 2.0 },
            jitter_strategy: JitterStrategy::Partial,
            should_retry_fn: None,
        }
    }
}

impl RetryConfig {
    /// Create a new retry configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum retry attempts
    pub fn max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Set base delay
    pub fn base_delay(mut self, delay: Duration) -> Self {
        self.base_delay = delay;
        self
    }

    /// Set maximum delay
    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Use exponential backoff strategy
    pub fn exponential_backoff(mut self, multiplier: f64) -> Self {
        self.backoff_strategy = BackoffStrategy::Exponential { multiplier };
        self
    }

    /// Use fixed backoff strategy
    pub fn fixed_backoff(mut self) -> Self {
        self.backoff_strategy = BackoffStrategy::Fixed;
        self
    }

    /// Use linear backoff strategy
    pub fn linear_backoff(mut self, increment: Duration) -> Self {
        self.backoff_strategy = BackoffStrategy::Linear { increment };
        self
    }

    /// Set jitter strategy
    pub fn jitter(mut self, strategy: JitterStrategy) -> Self {
        self.jitter_strategy = strategy;
        self
    }

    /// Set custom retry condition
    pub fn should_retry<F>(mut self, f: F) -> Self
    where
        F: Fn(&KodeBridgeError, usize) -> bool + Send + Sync + 'static,
    {
        self.should_retry_fn = Some(Box::new(f));
        self
    }

    /// Smart defaults for different scenarios
    pub fn for_network_operations() -> Self {
        Self::new()
            .max_attempts(5)
            .base_delay(Duration::from_millis(50))
            .max_delay(Duration::from_secs(10))
            .exponential_backoff(2.0)
            .jitter(JitterStrategy::Full)
    }

    pub fn for_rate_limited_apis() -> Self {
        Self::new()
            .max_attempts(10)
            .base_delay(Duration::from_secs(1))
            .max_delay(Duration::from_secs(60))
            .exponential_backoff(1.5)
            .jitter(JitterStrategy::Decorrelated)
    }

    pub fn for_quick_operations() -> Self {
        Self::new()
            .max_attempts(2)
            .base_delay(Duration::from_millis(10))
            .max_delay(Duration::from_millis(100))
            .fixed_backoff()
            .jitter(JitterStrategy::None)
    }

    /// Optimized configuration for PUT requests
    pub fn for_put_requests() -> Self {
        Self::new()
            .max_attempts(2) // 少重试，快速失败
            .base_delay(Duration::from_millis(25))
            .max_delay(Duration::from_millis(200))
            .exponential_backoff(1.5) // 温和的退避
            .jitter(JitterStrategy::Partial)
    }

    /// Configuration for large PUT requests
    pub fn for_large_put_requests() -> Self {
        Self::new()
            .max_attempts(3) // 稍多重试，因为大请求更容易失败
            .base_delay(Duration::from_millis(50))
            .max_delay(Duration::from_millis(500))
            .linear_backoff(Duration::from_millis(50))
            .jitter(JitterStrategy::Partial)
    }
}

/// Retry state tracking
#[derive(Debug)]
pub struct RetryState {
    attempt: usize,
    total_elapsed: Duration,
    last_delay: Duration,
}

impl Default for RetryState {
    fn default() -> Self {
        Self {
            attempt: 0,
            total_elapsed: Duration::ZERO,
            last_delay: Duration::ZERO,
        }
    }
}

impl RetryState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attempt(&self) -> usize {
        self.attempt
    }

    pub fn total_elapsed(&self) -> Duration {
        self.total_elapsed
    }

    pub fn last_delay(&self) -> Duration {
        self.last_delay
    }
}

/// Smart retry executor
pub struct RetryExecutor {
    config: RetryConfig,
}

impl RetryExecutor {
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Execute operation with retry logic
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> Result<T, KodeBridgeError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, KodeBridgeError>>,
    {
        let mut state = RetryState::new();
        let mut rng = StdRng::from_seed([0u8; 32]); // Use deterministic seed for Send compatibility

        loop {
            state.attempt += 1;
            let attempt_start = Instant::now();

            debug!("Retry attempt {} starting", state.attempt);

            match operation().await {
                Ok(result) => {
                    if state.attempt > 1 {
                        debug!(
                            "Operation succeeded on attempt {} after {}ms",
                            state.attempt,
                            state.total_elapsed.as_millis()
                        );
                    }
                    return Ok(result);
                }
                Err(error) => {
                    let attempt_duration = attempt_start.elapsed();
                    state.total_elapsed += attempt_duration;

                    // Check if we should retry this error
                    let should_retry = if let Some(ref custom_fn) = self.config.should_retry_fn {
                        custom_fn(&error, state.attempt)
                    } else {
                        self.default_should_retry(&error, state.attempt)
                    };

                    if !should_retry || state.attempt >= self.config.max_attempts {
                        warn!(
                            "Operation failed after {} attempts in {}ms: {}",
                            state.attempt,
                            state.total_elapsed.as_millis(),
                            error
                        );
                        return Err(error);
                    }

                    // Calculate next delay
                    let next_delay = self.calculate_delay(&mut state, &mut rng);

                    debug!(
                        "Retrying after {}ms (attempt {}/{}, error: {})",
                        next_delay.as_millis(),
                        state.attempt,
                        self.config.max_attempts,
                        error
                    );

                    tokio::time::sleep(next_delay).await;
                }
            }
        }
    }

    /// Execute operation with context for better error reporting
    pub async fn execute_with_context<F, Fut, T>(
        &self,
        operation_name: &str,
        operation: F,
    ) -> Result<T, KodeBridgeError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, KodeBridgeError>>,
    {
        debug!("Starting retry execution for operation: {}", operation_name);

        match self.execute(operation).await {
            Ok(result) => {
                debug!("Operation '{}' completed successfully", operation_name);
                Ok(result)
            }
            Err(error) => {
                warn!(
                    "Operation '{}' failed with error: {}",
                    operation_name, error
                );
                Err(KodeBridgeError::custom(format!(
                    "Operation '{}' failed after retries: {}",
                    operation_name, error
                )))
            }
        }
    }

    /// Default retry logic based on error type
    fn default_should_retry(&self, error: &KodeBridgeError, attempt: usize) -> bool {
        use KodeBridgeError::*;

        match error {
            // Always retry network-related errors
            Io(_) | Connection { .. } | Timeout { .. } | StreamClosed => true,

            // Retry server errors (5xx) but not client errors (4xx)
            ServerError { status } => *status >= 500,
            ClientError { .. } | InvalidRequest { .. } => false,

            // Don't retry parsing or protocol errors
            HttpParse(_) | Http(_) | Protocol { .. } => false,

            // Don't retry configuration or validation errors
            Configuration { .. } => false,

            // Don't retry JSON errors (likely application issue)
            Json(_) | JsonSerialize { .. } => false,

            // Don't retry UTF-8 errors
            Utf8(_) | FromUtf8(_) => false,

            // Retry resource exhaustion with exponential backoff
            PoolExhausted => attempt <= 5, // But limit attempts for pool exhaustion

            // Custom errors - be conservative
            Custom { .. } => false,

            // HTTP status code errors need special handling
            InvalidStatusCode(_) => false,
        }
    }

    /// Calculate next retry delay with backoff and jitter
    fn calculate_delay(&self, state: &mut RetryState, _rng: &mut impl rand::Rng) -> Duration {
        let base_delay = match self.config.backoff_strategy {
            BackoffStrategy::Fixed => self.config.base_delay,
            BackoffStrategy::Exponential { multiplier } => {
                if state.attempt == 1 {
                    self.config.base_delay
                } else {
                    let exponential = (self.config.base_delay.as_millis() as f64
                        * multiplier.powi((state.attempt - 1) as i32))
                        as u64;
                    Duration::from_millis(exponential)
                }
            }
            BackoffStrategy::Linear { increment } => {
                self.config.base_delay + increment * (state.attempt as u32 - 1)
            }
        };

        // Cap at maximum delay
        let capped_delay = std::cmp::min(base_delay, self.config.max_delay);

        // Apply jitter
        let final_delay = match self.config.jitter_strategy {
            JitterStrategy::None => capped_delay,
            JitterStrategy::Full => {
                let jitter = random_range(0..=capped_delay.as_millis() / 2) as u64;
                capped_delay + Duration::from_millis(jitter)
            }
            JitterStrategy::Partial => {
                let jitter = random_range(0..=capped_delay.as_millis() / 4) as u64;
                capped_delay + Duration::from_millis(jitter)
            }
            JitterStrategy::Decorrelated => {
                // Decorrelated jitter: next_delay = random_between(base_delay, last_delay * 3)
                let min_delay = self.config.base_delay.as_millis() as u64;
                let max_delay = std::cmp::min(
                    (state.last_delay.as_millis() as u64 * 3).max(min_delay),
                    self.config.max_delay.as_millis() as u64,
                );
                Duration::from_millis(random_range(min_delay..=max_delay))
            }
        };

        state.last_delay = final_delay;
        final_delay
    }
}

/// Convenience function for simple retry operations
pub async fn retry<F, Fut, T>(config: RetryConfig, operation: F) -> Result<T, KodeBridgeError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, KodeBridgeError>>,
{
    RetryExecutor::new(config).execute(operation).await
}

/// Convenience function with default configuration
pub async fn retry_default<F, Fut, T>(operation: F) -> Result<T, KodeBridgeError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, KodeBridgeError>>,
{
    retry(RetryConfig::default(), operation).await
}

/// Circuit breaker pattern for failing services
#[derive(Debug)]
pub struct CircuitBreaker {
    failure_threshold: usize,
    recovery_timeout: Duration,
    consecutive_failures: usize,
    last_failure_time: Option<Instant>,
    state: CircuitState,
}

#[derive(Debug, Clone, PartialEq)]
enum CircuitState {
    Closed,   // Normal operation
    Open,     // Failing, reject requests
    HalfOpen, // Testing if service recovered
}

impl CircuitBreaker {
    pub fn new(failure_threshold: usize, recovery_timeout: Duration) -> Self {
        Self {
            failure_threshold,
            recovery_timeout,
            consecutive_failures: 0,
            last_failure_time: None,
            state: CircuitState::Closed,
        }
    }

    pub async fn execute<F, Fut, T>(&mut self, operation: F) -> Result<T, KodeBridgeError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, KodeBridgeError>>,
    {
        if self.state == CircuitState::Open {
            if let Some(last_failure) = self.last_failure_time {
                if last_failure.elapsed() >= self.recovery_timeout {
                    debug!("Circuit breaker entering half-open state");
                    self.state = CircuitState::HalfOpen;
                } else {
                    return Err(KodeBridgeError::custom("Circuit breaker is open"));
                }
            } else {
                return Err(KodeBridgeError::custom("Circuit breaker is open"));
            }
        }

        match operation().await {
            Ok(result) => {
                // Success - reset circuit breaker
                if self.state == CircuitState::HalfOpen {
                    debug!("Circuit breaker closing after successful operation");
                }
                self.consecutive_failures = 0;
                self.last_failure_time = None;
                self.state = CircuitState::Closed;
                Ok(result)
            }
            Err(error) => {
                // Failure - update circuit breaker state
                self.consecutive_failures += 1;
                self.last_failure_time = Some(Instant::now());

                if self.consecutive_failures >= self.failure_threshold {
                    debug!(
                        "Circuit breaker opening after {} consecutive failures",
                        self.consecutive_failures
                    );
                    self.state = CircuitState::Open;
                }

                Err(error)
            }
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, CircuitState::Open)
    }

    pub fn reset(&mut self) {
        self.consecutive_failures = 0;
        self.last_failure_time = None;
        self.state = CircuitState::Closed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let config = RetryConfig::new().max_attempts(3);
        let executor = RetryExecutor::new(config);

        let result = executor
            .execute(|| async { Ok::<i32, KodeBridgeError>(42) })
            .await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
        let config = RetryConfig::new()
            .max_attempts(3)
            .base_delay(Duration::from_millis(1));
        let executor = RetryExecutor::new(config);
        let attempt_count = Arc::new(AtomicUsize::new(0));

        let result = executor
            .execute(|| {
                let count = attempt_count.clone();
                async move {
                    let current = count.fetch_add(1, Ordering::SeqCst);
                    if current < 2 {
                        Err(KodeBridgeError::connection("Temporary failure"))
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_max_attempts_exceeded() {
        let config = RetryConfig::new()
            .max_attempts(2)
            .base_delay(Duration::from_millis(1));
        let executor = RetryExecutor::new(config);
        let attempt_count = Arc::new(AtomicUsize::new(0));

        let result = executor
            .execute(|| {
                let count = attempt_count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>(KodeBridgeError::connection("Always fails"))
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(attempt_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_non_retriable_error() {
        let config = RetryConfig::new()
            .max_attempts(3)
            .base_delay(Duration::from_millis(1));
        let executor = RetryExecutor::new(config);
        let attempt_count = Arc::new(AtomicUsize::new(0));

        let result = executor
            .execute(|| {
                let count = attempt_count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, _>(KodeBridgeError::ClientError { status: 400 })
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(attempt_count.load(Ordering::SeqCst), 1); // No retry for client error
    }

    #[tokio::test]
    async fn test_circuit_breaker() {
        let mut breaker = CircuitBreaker::new(2, Duration::from_millis(100));

        // First failure
        let result = breaker
            .execute(|| async { Err::<i32, _>(KodeBridgeError::connection("Failure 1")) })
            .await;
        assert!(result.is_err());
        assert!(!breaker.is_open());

        // Second failure - should open circuit
        let result = breaker
            .execute(|| async { Err::<i32, _>(KodeBridgeError::connection("Failure 2")) })
            .await;
        assert!(result.is_err());
        assert!(breaker.is_open());

        // Third attempt should be rejected immediately
        let result = breaker
            .execute(|| async { Ok::<i32, KodeBridgeError>(42) })
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Circuit breaker is open"));
    }

    #[test]
    fn test_backoff_strategies() {
        let mut state = RetryState::new();
        let mut rng = StdRng::from_seed([0u8; 32]); // Use deterministic seed for Send compatibility

        // Test exponential backoff
        let config = RetryConfig::new()
            .exponential_backoff(2.0)
            .base_delay(Duration::from_millis(100))
            .jitter(JitterStrategy::None);
        let executor = RetryExecutor::new(config);

        state.attempt = 1;
        let delay1 = executor.calculate_delay(&mut state, &mut rng);
        assert_eq!(delay1, Duration::from_millis(100));

        state.attempt = 2;
        let delay2 = executor.calculate_delay(&mut state, &mut rng);
        assert_eq!(delay2, Duration::from_millis(200));

        state.attempt = 3;
        let delay3 = executor.calculate_delay(&mut state, &mut rng);
        assert_eq!(delay3, Duration::from_millis(400));
    }

    #[test]
    fn test_retry_config_builder() {
        let config = RetryConfig::for_network_operations();
        assert_eq!(config.max_attempts, 5);
        assert_eq!(config.base_delay, Duration::from_millis(50));

        let config = RetryConfig::for_rate_limited_apis();
        assert_eq!(config.max_attempts, 10);
        assert_eq!(config.base_delay, Duration::from_secs(1));
    }
}
