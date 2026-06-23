use crate::config::types::{ClientConfig, LoadBalancingStrategy};
use rand::prelude::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// 选择客户端的策略函数
pub fn select_clients(
    clients: Vec<ClientConfig>,
    routing_keys: Option<Vec<(String, usize)>>,
    strategy: &LoadBalancingStrategy,
    active_counts: Option<&HashMap<String, i64>>,
) -> Vec<ClientConfig> {
    if clients.is_empty() {
        return clients;
    }

    match strategy {
        LoadBalancingStrategy::Random => select_clients_by_random_weight(clients),
        LoadBalancingStrategy::Deterministic => {
            if let Some(keys) = routing_keys {
                if !keys.is_empty() {
                    return select_clients_by_voting(clients, keys);
                }
            }
            select_clients_by_random_weight(clients)
        }
        LoadBalancingStrategy::LeastConnections => {
            select_clients_by_least_connections(clients, active_counts)
        }
    }
}

/// 传统的加权随机算法 (Weighted Random Sampling)
fn select_clients_by_random_weight(clients: Vec<ClientConfig>) -> Vec<ClientConfig> {
    let mut rng = rand::thread_rng();
    let mut weighted_clients: Vec<(f64, ClientConfig)> = clients
        .into_iter()
        .map(|client| {
            let weight = client.priority.unwrap_or(0) as f64;
            if weight <= 0.0 {
                (0.0, client)
            } else {
                let random_value: f64 = rng.gen();
                let sort_key = random_value.powf(1.0 / weight);
                (sort_key, client)
            }
        })
        .collect();

    weighted_clients.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    weighted_clients
        .into_iter()
        .map(|(_, client)| client)
        .collect()
}

/// 基于多锚点加权投票的确定性路由算法
fn select_clients_by_voting(
    clients: Vec<ClientConfig>,
    keys: Vec<(String, usize)>,
) -> Vec<ClientConfig> {
    let mut client_scores: HashMap<String, u64> = HashMap::new();

    for (key_content, length_weight) in keys {
        let mut best_client_name = String::new();
        let mut max_hash_score = -1.0;

        for client in &clients {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            key_content.hash(&mut hasher);
            client.name.hash(&mut hasher);
            let hash_val = hasher.finish();

            let normalized_hash = ((hash_val as f64) + 1.0) / ((u64::MAX as f64) + 1.0);
            let priority = client.priority.unwrap_or(0) as f64;

            let score = if priority <= 0.0 {
                0.0
            } else {
                normalized_hash.powf(1.0 / priority)
            };

            if score > max_hash_score {
                max_hash_score = score;
                best_client_name = client.name.clone();
            }
        }

        if !best_client_name.is_empty() {
            let entry = client_scores.entry(best_client_name).or_insert(0);
            *entry += length_weight as u64;
        }
    }

    let mut ranked_clients: Vec<(u64, ClientConfig)> = clients
        .into_iter()
        .map(|c| {
            let score = *client_scores.get(&c.name).unwrap_or(&0);
            (score, c)
        })
        .collect();

    ranked_clients.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));

    ranked_clients.into_iter().map(|(_, c)| c).collect()
}

/// 加权最少连接路由算法
fn select_clients_by_least_connections(
    clients: Vec<ClientConfig>,
    active_counts: Option<&HashMap<String, i64>>,
) -> Vec<ClientConfig> {
    let empty_map = HashMap::new();
    let counts = active_counts.unwrap_or(&empty_map);

    let mut scored_clients: Vec<(f64, ClientConfig)> = clients
        .into_iter()
        .map(|client| {
            let active = *counts.get(&client.name).unwrap_or(&0);
            let priority = client.priority.unwrap_or(1).max(1) as f64;
            let score = if active <= 0 {
                0.0_f64
            } else {
                active as f64 / priority
            };
            (score, client)
        })
        .collect();

    scored_clients.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.name.cmp(&a.1.name))
    });

    scored_clients.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{ClientConfig, ModelMatch};

    fn create_test_client(name: &str, priority: u32) -> ClientConfig {
        ClientConfig {
            name: name.to_string(),
            base_url: "http://example.com".to_string(),
            api_key: None,
            model_match: ModelMatch {
                match_type: "exact".to_string(),
                value: vec!["test-model".to_string()],
            },
            priority: Some(priority),
            fallback: None,
            special_prefix: None,
            stop: None,
            max_tokens: None,
            extra_body: None,
        }
    }

    #[test]
    fn test_select_clients_by_random_weight() {
        let clients = vec![
            create_test_client("client1", 1),
            create_test_client("client2", 3),
            create_test_client("client3", 2),
        ];

        let selected = select_clients_by_random_weight(clients);
        assert_eq!(selected.len(), 3);
    }

    #[test]
    fn test_select_clients_by_random_weight_empty() {
        let clients: Vec<ClientConfig> = vec![];
        let selected = select_clients_by_random_weight(clients);
        assert!(selected.is_empty());
    }

    #[test]
    fn test_select_clients_with_routing_keys() {
        let clients = vec![
            create_test_client("client1", 10),
            create_test_client("client2", 10),
        ];
        let strategy = LoadBalancingStrategy::Deterministic;
        let routing_keys = vec![("test content here".to_string(), 100)];
        let selected = select_clients(clients.clone(), Some(routing_keys), &strategy, None);
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_select_clients_empty_input() {
        let clients: Vec<ClientConfig> = vec![];
        let selected = select_clients(clients, None, &LoadBalancingStrategy::Deterministic, None);
        assert!(selected.is_empty());
    }

    #[test]
    fn test_select_clients_single_client() {
        let clients = vec![create_test_client("client1", 5)];
        let selected = select_clients(clients, None, &LoadBalancingStrategy::Deterministic, None);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "client1");
    }

    #[test]
    fn test_deterministic_routing_same_input() {
        let clients = vec![
            create_test_client("server1", 10),
            create_test_client("server2", 10),
        ];
        let routing_keys = vec![("hello world".to_string(), 50)];

        let result1 = select_clients_by_voting(clients.clone(), routing_keys.clone());
        let result2 = select_clients_by_voting(clients, routing_keys);

        assert_eq!(result1[0].name, result2[0].name);
    }

    #[test]
    fn test_deterministic_routing_different_content() {
        let clients = vec![
            create_test_client("server1", 10),
            create_test_client("server2", 10),
        ];
        let keys1 = vec![("content A".to_string(), 50)];
        let keys2 = vec![("content B".to_string(), 50)];

        let result1 = select_clients_by_voting(clients.clone(), keys1);
        let result2 = select_clients_by_voting(clients, keys2);

        assert!(
            result1[0].name != result2[0].name
                || result1[0].name == "server1" && result2[0].name == "server1"
        );
    }

    #[test]
    fn test_least_connections_empty_counts() {
        let clients = vec![
            create_test_client("client1", 10),
            create_test_client("client2", 5),
        ];
        let selected = select_clients_by_least_connections(clients, None);
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_least_connections_with_counts() {
        let clients = vec![
            create_test_client("client1", 10),
            create_test_client("client2", 5),
            create_test_client("client3", 1),
        ];
        let mut counts = HashMap::new();
        counts.insert("client1".to_string(), 50);
        counts.insert("client2".to_string(), 5);
        counts.insert("client3".to_string(), 0);

        let selected = select_clients_by_least_connections(clients, Some(&counts));
        assert_eq!(selected.len(), 3);
        // client3 has 0 active -> score 0 -> first
        assert_eq!(selected[0].name, "client3");
        // client2 has 5/5 = 1.0 -> second
        assert_eq!(selected[1].name, "client2");
        // client1 has 50/10 = 5.0 -> third
        assert_eq!(selected[2].name, "client1");
    }

    #[test]
    fn test_random_strategy_ignores_routing_keys() {
        let clients = vec![
            create_test_client("client1", 1),
            create_test_client("client2", 1),
        ];
        let keys = Some(vec![("hello".to_string(), 10)]);
        let selected = select_clients(clients, keys, &LoadBalancingStrategy::Random, None);
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_deterministic_fallback_to_random() {
        let clients = vec![
            create_test_client("client1", 1),
            create_test_client("client2", 1),
        ];
        let selected = select_clients(clients, None, &LoadBalancingStrategy::Deterministic, None);
        assert_eq!(selected.len(), 2);
    }
}
