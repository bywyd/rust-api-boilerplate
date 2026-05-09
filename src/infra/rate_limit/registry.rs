use crate::infra::config::app_config::RateLimitConfig;
use actix_governor::governor::clock::QuantaInstant;
use actix_governor::governor::middleware::NoOpMiddleware;
use actix_governor::{GovernorConfig, GovernorConfigBuilder, Governor, PeerIpKeyExtractor};
use actix_web::middleware::Condition;
use std::collections::HashMap;
use std::sync::Arc;

/// Concrete governor-config type: IP-keyed, no-op middleware, quanta clock.
type GovConfig = GovernorConfig<PeerIpKeyExtractor, NoOpMiddleware<QuantaInstant>>;

/// Concrete governor middleware type (same params as `GovConfig`).
pub type GovMiddleware = Governor<PeerIpKeyExtractor, NoOpMiddleware<QuantaInstant>>;

/// A registry of named IP-based rate-limit rules, built once at startup.
///
/// Each rule stores its own [`GovConfig`] which internally holds an
/// `Arc<RateLimiter>`. Cloning the `Arc<RateLimitRegistry>` across actix
/// worker threads therefore shares the same counters — i.e., limits are
/// enforced globally, not per-worker.
///
/// # Defining rules
///
/// Rules are declared under `rate_limit.rules` in your YAML config:
///
/// ```yaml
/// rate_limit:
///   enabled: true
///   rules:
///     default:
///       seconds_per_request: 1    # sustain 1 req/s per IP
///       burst_size: 60            # allow short bursts up to 60 requests
///     auth:
///       seconds_per_request: 20   # 1 req per 20 s for login endpoints
///       burst_size: 5
///     strict:
///       seconds_per_request: 60
///       burst_size: 2
/// ```
///
/// # Applying rules in `router.rs`
///
/// ```rust,ignore
/// web::scope("/auth")
///     .wrap(state.rate_limit.condition("auth"))
///
/// web::scope("/users")
///     .wrap(state.rate_limit.condition("default"))
/// ```
pub struct RateLimitRegistry {
    rules: HashMap<String, Arc<GovConfig>>,
    enabled: bool,
}

impl RateLimitRegistry {
    /// Build the registry from the application's `RateLimitConfig`.
    ///
    /// Every rule in `cfg.rules` is compiled into a [`GovConfig`] immediately;
    /// the resulting `Arc<RateLimiter>` is held for the lifetime of the process.
    pub fn from_config(cfg: &RateLimitConfig) -> Self {
        let mut rules = HashMap::new();

        for (name, rule) in &cfg.rules {
            let conf = GovernorConfigBuilder::default()
                .seconds_per_request(rule.seconds_per_request)
                .burst_size(rule.burst_size)
                .finish()
                .unwrap_or_else(|| {
                    panic!(
                        "Rate limit rule '{name}' is invalid — check \
                         seconds_per_request ({}) and burst_size ({}).",
                        rule.seconds_per_request, rule.burst_size
                    )
                });

            rules.insert(name.clone(), Arc::new(conf));
        }

        tracing::info!(
            enabled = cfg.enabled,
            rules = ?rules.keys().collect::<Vec<_>>(),
            "Rate limit registry initialised"
        );

        Self {
            rules,
            enabled: cfg.enabled,
        }
    }

    /// Returns a [`Condition<GovMiddleware>`] that applies the named rule only
    /// when rate limiting is globally enabled (`rate_limit.enabled = true`).
    ///
    /// Use this on route scopes that should respect the global kill-switch:
    ///
    /// ```rust,ignore
    /// web::scope("/auth").wrap(state.rate_limit.condition("auth"))
    /// ```
    ///
    /// # Panics
    ///
    /// Panics at startup if `name` is not registered — catches configuration
    /// mistakes at boot time rather than silently running unprotected.
    pub fn condition(&self, name: &str) -> Condition<GovMiddleware> {
        Condition::new(self.enabled, self.governor(name))
    }

    /// Returns an unconditional [`GovMiddleware`] for the named rule,
    /// regardless of the global `enabled` flag.
    ///
    /// Use this when a route *must* always be protected (e.g. a payment
    /// endpoint) even if the global switch is off.
    ///
    /// # Panics
    ///
    /// Panics at startup if `name` is not registered.
    pub fn governor(&self, name: &str) -> GovMiddleware {
        let conf = self.rules.get(name).unwrap_or_else(|| {
            panic!(
                "Rate limit rule '{name}' is not registered. \
                 Add it under `rate_limit.rules` in your config."
            )
        });
        Governor::new(conf)
    }

    /// `true` when the global `rate_limit.enabled` flag is set.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Iterate over every registered rule name.
    pub fn rule_names(&self) -> impl Iterator<Item = &str> {
        self.rules.keys().map(String::as_str)
    }
}
