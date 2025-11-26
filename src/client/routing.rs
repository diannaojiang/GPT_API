use crate::config::types::ClientConfig;
use rand::prelude::*;

pub fn select_clients_by_weight(clients: Vec<ClientConfig>) -> Vec<ClientConfig> {
    if clients.is_empty() {
        return clients;
    }

    let mut rng = rand::thread_rng();
    let mut weighted_clients: Vec<(f64, ClientConfig)> = clients
        .into_iter()
        .map(|client| {
            let weight = client.priority.unwrap_or(0) as f64;
            if weight <= 0.0 {
                // Assign a sort key that will place it at the end.
                // A key of 0.0 is the lowest possible for this algorithm.
                (0.0, client)
            } else {
                // Use `gen` for a value in [0, 1)
                let random_value: f64 = rng.gen();
                // The sort key is U^(1/w).
                let sort_key = random_value.powf(1.0 / weight);
                (sort_key, client)
            }
        })
        .collect();

    // Sort by the key in descending order. Higher key means higher priority.
    weighted_clients.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Extract the sorted clients.
    weighted_clients
        .into_iter()
        .map(|(_, client)| client)
        .collect()
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
        }
    }

    #[test]
    fn test_select_clients_by_weight() {
        let clients = vec![
            create_test_client("client1", 1),
            create_test_client("client2", 3),
            create_test_client("client3", 2),
        ];

        let selected = select_clients_by_weight(clients);

        // 检查返回的客户端数量是否正确
        assert_eq!(selected.len(), 3);

        // 由于是随机排序，我们不能断言具体的顺序
        // 但我们可以通过多次运行来验证分布
    }

    #[test]
    fn test_select_clients_by_weight_empty() {
        let clients: Vec<ClientConfig> = vec![];
        let selected = select_clients_by_weight(clients);
        assert!(selected.is_empty());
    }
}
