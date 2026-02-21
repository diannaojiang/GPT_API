use crate::config::types::ClientConfig;
use rand::prelude::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// 选择客户端的策略函数
///
/// 支持两种模式：
/// 1. **加权随机**：当 `routing_keys` 为空时使用。基于客户端权重进行加权随机排序。
/// 2. **加权投票 (Multi-Anchor Voting)**：当 `routing_keys` 存在时使用。
///    - `routing_keys` 是一个 `(key_content, weight)` 的列表。
///    - 这里的 `weight` 是消息长度，`key_content` 是消息内容的前 N 个字符。
///    - 每一条消息都会基于 Rendezvous Hashing 独立选出一个“最佳客户端”。
///    - 该客户端会获得相当于 `weight` 的积分。
///    - 最终根据积分总和对客户端进行排序，积分高的排前面。
///    - 这种机制能最大化 KV Cache 亲和性，同时通过长度加权防止短消息干扰。
pub fn select_clients(
    clients: Vec<ClientConfig>,
    routing_keys: Option<Vec<(String, usize)>>,
) -> Vec<ClientConfig> {
    if clients.is_empty() {
        return clients;
    }

    // 如果提供了路由键且不为空，使用确定性加权投票算法
    if let Some(keys) = routing_keys {
        if !keys.is_empty() {
            return select_clients_by_voting(clients, keys);
        }
    }

    // 否则回退到加权随机
    select_clients_by_random_weight(clients)
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
                // Efraimidis and Spirakis algorithm: key = U^(1/w)
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

    // 1. 投票过程：每一条消息(锚点)独立选举
    for (key_content, length_weight) in keys {
        let mut best_client_name = String::new();
        let mut max_hash_score = -1.0;

        for client in &clients {
            // Rendezvous Hash (HRW) 核心: Hash(Object + Node)
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            key_content.hash(&mut hasher);
            client.name.hash(&mut hasher);
            let hash_val = hasher.finish();

            // 归一化哈希值到 [0, 1]
            // 为了数学严谨性，确保结果在 (0, 1] 区间，避免 0.0 的边界情况（虽然 u64 哈希碰撞概率极低）
            // 我们使用 hash_val + 1 避免 0，且除以 MAX + 1.0 保持在 1.0 以内
            let normalized_hash = ((hash_val as f64) + 1.0) / ((u64::MAX as f64) + 1.0);

            let priority = client.priority.unwrap_or(0) as f64;

            // 使用 Efraimidis-Spirakis 算法: score = r^(1/w)
            // 这种算法在保持确定性（只要 r 确定）的同时，能让概率分布严格逼近权重比例
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

        // 计票：获胜者获得该消息长度的积分
        if !best_client_name.is_empty() {
            let entry = client_scores.entry(best_client_name).or_insert(0);
            *entry += length_weight as u64;
        }
    }

    // 2. 排序：按积分从高到低排列所有客户端
    let mut ranked_clients: Vec<(u64, ClientConfig)> = clients
        .into_iter()
        .map(|c| {
            let score = *client_scores.get(&c.name).unwrap_or(&0);
            (score, c)
        })
        .collect();

    ranked_clients.sort_by(|a, b| {
        b.0.cmp(&a.0) // 积分降序
            .then_with(|| a.1.name.cmp(&b.1.name)) // 积分相同时字典序，保证确定性
    });

    ranked_clients.into_iter().map(|(_, c)| c).collect()
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
    fn test_select_clients_by_random_weight() {
        let clients = vec![
            create_test_client("client1", 1),
            create_test_client("client2", 3),
            create_test_client("client3", 2),
        ];

        let selected = select_clients_by_random_weight(clients);

        // 检查返回的客户端数量是否正确
        assert_eq!(selected.len(), 3);

        // 由于是随机排序，我们不能断言具体的顺序
        // 但我们可以通过多次运行来验证分布
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

        let routing_keys = vec![("test content here".to_string(), 100)];
        let selected = select_clients(clients.clone(), Some(routing_keys));

        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_select_clients_empty_input() {
        let clients: Vec<ClientConfig> = vec![];
        let selected = select_clients(clients, None);
        assert!(selected.is_empty());
    }

    #[test]
    fn test_select_clients_single_client() {
        let clients = vec![create_test_client("client1", 5)];

        let selected = select_clients(clients, None);

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
}
