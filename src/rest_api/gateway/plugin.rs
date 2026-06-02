//! Plugin System for API Gateway
//!
//! Provides an extensible plugin architecture for the gateway
//! with support for custom authentication, rate limiting, transformation,
//! and analytics plugins.

use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use axum::{
    body::Body,
    extract::Request,
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::controller::ControllerState;
use super::auth::AuthContext;

/// Plugin hook types
#[derive(Debug, Clone)]
pub enum PluginHook {
    /// Called before request processing
    PreRequest,
    /// Called after request processing but before response
    PostRequest,
    /// Called after response is generated
    PostResponse,
    /// Called on request error
    OnError,
    /// Called periodically for background tasks
    Periodic,
}

/// Plugin configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSettings {
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub config: HashMap<String, String>,
    pub hooks: Vec<String>,
}

/// Plugin trait - implement this to create custom plugins
#[async_trait]
pub trait GatewayPlugin: Send + Sync {
    /// Plugin name
    fn name(&self) -> &str;

    /// Plugin version
    fn version(&self) -> &str;

    /// Initialize plugin with configuration
    async fn init(&mut self, config: HashMap<String, String>) -> Result<(), String>;

    /// Get enabled hooks
    fn hooks(&self) -> Vec<PluginHook>;

    /// Pre-request hook - return None to continue, Some(false) to reject
    async fn pre_request(&self, ctx: &mut PluginContext) -> Option<bool>;

    /// Post-request hook
    async fn post_request(&self, ctx: &mut PluginContext);

    /// Post-response hook
    async fn post_response(&self, ctx: &mut PluginContext);

    /// Error hook
    async fn on_error(&self, ctx: &mut PluginContext, error: &str);

    /// Periodic hook - called at regular intervals
    async fn periodic(&self, _ctx: &PluginContext) {}
}

/// Plugin execution context
#[derive(Debug)]
pub struct PluginContext {
    pub request: Request<Body>,
    pub auth: AuthContext,
    pub state: Arc<ControllerState>,
    pub response: Option<Response<Body>>,
    pub metadata: HashMap<String, String>,
    pub start_time: DateTime<Utc>,
}

impl PluginContext {
    pub fn new(request: Request<Body>, auth: AuthContext, state: Arc<ControllerState>) -> Self {
        Self {
            request,
            auth,
            state,
            response: None,
            metadata: HashMap::new(),
            start_time: Utc::now(),
        }
    }

    /// Get metadata value
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }

    /// Set metadata value
    pub fn set_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }
}

/// Plugin manager - handles plugin lifecycle
pub struct PluginManager {
    plugins: Arc<RwLock<HashMap<String, Box<dyn GatewayPlugin>>>>,
    settings: Arc<RwLock<HashMap<String, PluginSettings>>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            settings: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a plugin
    pub async fn register<P: GatewayPlugin + 'static>(&self, plugin: P) {
        let name = plugin.name().to_string();
        let mut plugins = self.plugins.write().await;
        
        // Initialize with default settings
        let settings = PluginSettings {
            name: name.clone(),
            version: plugin.version().to_string(),
            enabled: true,
            config: HashMap::new(),
            hooks: plugin.hooks().iter().map(|h| format!("{:?}", h)).collect(),
        };
        
        let mut plugin = plugin;
        if let Err(e) = plugin.init(HashMap::new()).await {
            tracing::warn!("Plugin {} init error: {}", name, e);
        }
        
        plugins.insert(name, Box::new(plugin));
        
        drop(plugins);
        let mut settings = self.settings.write().await;
        settings.insert(name, settings);
    }

    /// Unregister a plugin
    pub async fn unregister(&self, name: &str) -> bool {
        let mut plugins = self.plugins.write().await;
        plugins.remove(name).is_some()
    }

    /// Enable/disable a plugin
    pub async fn set_enabled(&self, name: &str, enabled: bool) {
        let mut settings = self.settings.write().await;
        if let Some(config) = settings.get_mut(name) {
            config.enabled = enabled;
        }
    }

    /// Update plugin configuration
    pub async fn update_config(&self, name: &str, config: HashMap<String, String>) -> Result<(), String> {
        let mut plugins = self.plugins.write().await;
        if let Some(plugin) = plugins.get_mut(name) {
            plugin.init(config).await
        } else {
            Err("Plugin not found".to_string())
        }
    }

    /// Pre-process a request
    pub async fn pre_process(&self, ctx: &mut PluginContext) -> Option<bool> {
        let settings = self.settings.read().await;
        let plugins = self.plugins.read().await;

        for (name, plugin) in plugins.iter() {
            if let Some(config) = settings.get(name) {
                if !config.enabled {
                    continue;
                }
            }

            if plugin.hooks().contains(&PluginHook::PreRequest) {
                if let Some(result) = plugin.pre_request(ctx).await {
                    if !result {
                        tracing::info!("Plugin {} rejected request", plugin.name());
                    }
                    return Some(result);
                }
            }
        }

        None
    }

    /// Post-process a request
    pub async fn post_process(&self, ctx: &mut PluginContext) {
        let settings = self.settings.read().await;
        let plugins = self.plugins.read().await;

        for (name, plugin) in plugins.iter() {
            if let Some(config) = settings.get(name) {
                if !config.enabled {
                    continue;
                }
            }

            if plugin.hooks().contains(&PluginHook::PostRequest) {
                plugin.post_request(ctx).await;
            }

            if let Some(ref response) = ctx.response {
                if plugin.hooks().contains(&PluginHook::PostResponse) {
                    plugin.post_response(ctx).await;
                }
            }
        }
    }

    /// Handle error
    pub async fn handle_error(&self, ctx: &mut PluginContext, error: &str) {
        let settings = self.settings.read().await;
        let plugins = self.plugins.read().await;

        for (name, plugin) in plugins.iter() {
            if let Some(config) = settings.get(name) {
                if !config.enabled {
                    continue;
                }
            }

            if plugin.hooks().contains(&PluginHook::OnError) {
                plugin.on_error(ctx, error).await;
            }
        }
    }

    /// Get list of registered plugins
    pub async fn list_plugins(&self) -> Vec<PluginSettings> {
        let settings = self.settings.read().await;
        settings.values().cloned().collect()
    }

    /// Get plugin info
    pub async fn get_plugin(&self, name: &str) -> Option<PluginSettings> {
        let settings = self.settings.read().await;
        settings.get(name).cloned()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Example: Custom authentication plugin
pub struct CustomAuthPlugin {
    name: String,
    version: String,
    allowed_tokens: Vec<String>,
}

impl CustomAuthPlugin {
    pub fn new() -> Self {
        Self {
            name: "custom-auth".to_string(),
            version: "1.0.0".to_string(),
            allowed_tokens: vec![],
        }
    }
}

#[async_trait]
impl GatewayPlugin for CustomAuthPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        &self.version
    }

    async fn init(&mut self, config: HashMap<String, String>) -> Result<(), String> {
        if let Some(tokens) = config.get("allowed_tokens") {
            self.allowed_tokens = tokens.split(',').map(|s| s.trim().to_string()).collect();
        }
        Ok(())
    }

    fn hooks(&self) -> Vec<PluginHook> {
        vec![PluginHook::PreRequest]
    }

    async fn pre_request(&self, ctx: &mut PluginContext) -> Option<bool> {
        // Custom auth logic here
        let token = ctx.request
            .headers()
            .get("X-Custom-Token")
            .and_then(|v| v.to_str().ok());

        if let Some(token) = token {
            if self.allowed_tokens.contains(&token.to_string()) {
                return Some(true);
            }
        }

        Some(false)
    }

    async fn post_request(&self, _ctx: &mut PluginContext) {}
    async fn post_response(&self, _ctx: &mut PluginContext) {}
    async fn on_error(&self, _ctx: &mut PluginContext, _error: &str) {}
}

/// Example: Request logging plugin
pub struct LoggingPlugin {
    name: String,
    version: String,
    log_body: bool,
}

impl LoggingPlugin {
    pub fn new() -> Self {
        Self {
            name: "request-logger".to_string(),
            version: "1.0.0".to_string(),
            log_body: false,
        }
    }
}

#[async_trait]
impl GatewayPlugin for LoggingPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        &self.version
    }

    async fn init(&mut self, config: HashMap<String, String>) -> Result<(), String> {
        if let Some(v) = config.get("log_body") {
            self.log_body = v == "true";
        }
        Ok(())
    }

    fn hooks(&self) -> Vec<PluginHook> {
        vec![PluginHook::PreRequest, PluginHook::PostResponse]
    }

    async fn pre_request(&self, ctx: &mut PluginContext) -> Option<bool> {
        tracing::info!(
            "Request: {} {} from {}",
            ctx.request.method(),
            ctx.request.uri(),
            ctx.auth.client_id
        );
        None
    }

    async fn post_request(&self, _ctx: &mut PluginContext) {}

    async fn post_response(&self, ctx: &mut PluginContext) {
        if let Some(response) = &ctx.response {
            let duration = (Utc::now() - ctx.start_time).num_milliseconds();
            tracing::info!(
                "Response: {} in {}ms",
                response.status(),
                duration
            );
        }
    }

    async fn on_error(&self, _ctx: &mut PluginContext, error: &str) {
        tracing::error!("Request error: {}", error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_plugin_manager_register() {
        let manager = PluginManager::new();
        
        let plugin = LoggingPlugin::new();
        manager.register(plugin).await;
        
        let plugins = manager.list_plugins().await;
        assert_eq!(plugins.len(), 1);
    }

    #[tokio::test]
    async fn test_plugin_manager_unregister() {
        let manager = PluginManager::new();
        
        let plugin = LoggingPlugin::new();
        manager.register(plugin).await;
        
        let removed = manager.unregister("request-logger").await;
        assert!(removed);
        
        let plugins = manager.list_plugins().await;
        assert!(plugins.is_empty());
    }

    #[tokio::test]
    async fn test_custom_auth_plugin() {
        let mut plugin = CustomAuthPlugin::new();
        
        let mut config = HashMap::new();
        config.insert("allowed_tokens".to_string(), "token1,token2".to_string());
        
        plugin.init(config).await.unwrap();
        
        assert!(plugin.hooks().contains(&PluginHook::PreRequest));
    }
}