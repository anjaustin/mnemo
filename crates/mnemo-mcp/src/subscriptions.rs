//! Resource subscription management for MCP.
//!
//! Tracks which resource URIs are subscribed to and provides
//! notification helpers. Subscriptions are session-scoped.

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::protocol::JsonRpcNotification;

/// Manages resource subscriptions.
#[derive(Debug, Default)]
pub struct SubscriptionManager {
    /// Set of subscribed resource URIs.
    subscriptions: RwLock<HashSet<String>>,
}

impl SubscriptionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to a resource URI.
    ///
    /// Returns `true` if this is a new subscription.
    pub async fn subscribe(&self, uri: &str) -> bool {
        let mut subs = self.subscriptions.write().await;
        subs.insert(uri.to_string())
    }

    /// Unsubscribe from a resource URI.
    ///
    /// Returns `true` if the subscription existed.
    pub async fn unsubscribe(&self, uri: &str) -> bool {
        let mut subs = self.subscriptions.write().await;
        subs.remove(uri)
    }

    /// Check if a resource URI is subscribed.
    pub async fn is_subscribed(&self, uri: &str) -> bool {
        let subs = self.subscriptions.read().await;
        subs.contains(uri)
    }

    /// Get all subscribed URIs.
    pub async fn list(&self) -> Vec<String> {
        let subs = self.subscriptions.read().await;
        subs.iter().cloned().collect()
    }

    /// Get the number of subscriptions.
    pub async fn count(&self) -> usize {
        let subs = self.subscriptions.read().await;
        subs.len()
    }

    /// Clear all subscriptions.
    pub async fn clear(&self) {
        let mut subs = self.subscriptions.write().await;
        subs.clear();
    }

    /// Check if a notification should be sent for a URI.
    ///
    /// This checks if the URI matches any subscribed pattern.
    /// Currently uses exact matching, but could support wildcards.
    pub async fn should_notify(&self, uri: &str) -> bool {
        self.is_subscribed(uri).await
    }
}

/// Shared subscription state for the MCP server.
pub type SharedSubscriptions = Arc<SubscriptionManager>;

/// Create a new shared subscription manager.
pub fn new_shared() -> SharedSubscriptions {
    Arc::new(SubscriptionManager::new())
}

/// Helper to create a resource updated notification.
pub fn resource_updated_notification(uri: &str) -> JsonRpcNotification {
    JsonRpcNotification::resource_updated(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_subscribe_new_uri() {
        let manager = SubscriptionManager::new();
        assert!(manager.subscribe("mnemo://users/alice/memory").await);
        assert!(manager.is_subscribed("mnemo://users/alice/memory").await);
    }

    #[tokio::test]
    async fn test_subscribe_duplicate_uri() {
        let manager = SubscriptionManager::new();
        assert!(manager.subscribe("mnemo://users/alice/memory").await);
        // Second subscribe returns false (already subscribed)
        assert!(!manager.subscribe("mnemo://users/alice/memory").await);
    }

    #[tokio::test]
    async fn test_unsubscribe_existing() {
        let manager = SubscriptionManager::new();
        manager.subscribe("mnemo://users/alice/memory").await;
        assert!(manager.unsubscribe("mnemo://users/alice/memory").await);
        assert!(!manager.is_subscribed("mnemo://users/alice/memory").await);
    }

    #[tokio::test]
    async fn test_unsubscribe_nonexistent() {
        let manager = SubscriptionManager::new();
        assert!(!manager.unsubscribe("mnemo://users/alice/memory").await);
    }

    #[tokio::test]
    async fn test_list_subscriptions() {
        let manager = SubscriptionManager::new();
        manager.subscribe("mnemo://users/alice/memory").await;
        manager.subscribe("mnemo://users/bob/memory").await;

        let list = manager.list().await;
        assert_eq!(list.len(), 2);
        assert!(list.contains(&"mnemo://users/alice/memory".to_string()));
        assert!(list.contains(&"mnemo://users/bob/memory".to_string()));
    }

    #[tokio::test]
    async fn test_count_subscriptions() {
        let manager = SubscriptionManager::new();
        assert_eq!(manager.count().await, 0);
        manager.subscribe("mnemo://users/alice/memory").await;
        assert_eq!(manager.count().await, 1);
        manager.subscribe("mnemo://users/bob/memory").await;
        assert_eq!(manager.count().await, 2);
    }

    #[tokio::test]
    async fn test_clear_subscriptions() {
        let manager = SubscriptionManager::new();
        manager.subscribe("mnemo://users/alice/memory").await;
        manager.subscribe("mnemo://users/bob/memory").await;
        assert_eq!(manager.count().await, 2);

        manager.clear().await;
        assert_eq!(manager.count().await, 0);
    }

    #[tokio::test]
    async fn test_should_notify_exact_match() {
        let manager = SubscriptionManager::new();
        manager.subscribe("mnemo://users/alice/memory").await;

        assert!(manager.should_notify("mnemo://users/alice/memory").await);
        assert!(!manager.should_notify("mnemo://users/bob/memory").await);
    }

    #[tokio::test]
    async fn test_shared_subscriptions() {
        let shared = new_shared();
        shared.subscribe("mnemo://users/test/memory").await;
        assert!(shared.is_subscribed("mnemo://users/test/memory").await);
    }

    #[test]
    fn test_resource_updated_notification() {
        let notification = resource_updated_notification("mnemo://users/alice/memory");
        assert_eq!(notification.jsonrpc, "2.0");
        assert_eq!(notification.method, "notifications/resources/updated");
        assert!(notification.params.is_some());
    }
}
