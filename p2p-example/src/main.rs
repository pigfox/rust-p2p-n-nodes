/// Validate and parse CLI arguments.
/// Returns Ok((node_id, total)) or an Err with a human-readable message.
pub fn parse_args(args: &[String]) -> Result<(usize, usize), String> {
    if args.len() < 3 {
        return Err(format!(
            "Usage: {} <node_id> <total_nodes>\n  node_id: 0-based index\n  total_nodes: cluster size",
            args.first().map(String::as_str).unwrap_or("p2p-example")
        ));
    }
    let node_id: usize = args[1]
        .parse()
        .map_err(|_| format!("node_id '{}' is not a valid number", args[1]))?;
    let total: usize = args[2]
        .parse()
        .map_err(|_| format!("total_nodes '{}' is not a valid number", args[2]))?;
    if node_id >= total {
        return Err(format!(
            "node_id ({node_id}) must be less than total_nodes ({total})"
        ));
    }
    Ok((node_id, total))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    let args: Vec<String> = std::env::args().collect();
    match parse_args(&args) {
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
        Ok((node_id, total)) => p2p_example::run_node(node_id, total).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_returns_error() {
        assert!(parse_args(&a(&["bin"])).is_err());
    }

    #[test]
    fn one_arg_returns_error() {
        assert!(parse_args(&a(&["bin", "0"])).is_err());
    }

    #[test]
    fn error_message_contains_usage() {
        let e = parse_args(&a(&["myprog"])).unwrap_err();
        assert!(e.contains("Usage"), "expected Usage in: {e}");
        assert!(e.contains("myprog"), "expected binary name in: {e}");
    }

    #[test]
    fn non_numeric_node_id_returns_error() {
        let e = parse_args(&a(&["bin", "abc", "3"])).unwrap_err();
        assert!(e.contains("abc"), "error should name the bad value: {e}");
    }

    #[test]
    fn non_numeric_total_returns_error() {
        let e = parse_args(&a(&["bin", "0", "xyz"])).unwrap_err();
        assert!(e.contains("xyz"), "error should name the bad value: {e}");
    }

    #[test]
    fn node_id_equal_to_total_returns_error() {
        let e = parse_args(&a(&["bin", "2", "2"])).unwrap_err();
        assert!(e.contains("less than"), "{e}");
    }

    #[test]
    fn node_id_greater_than_total_returns_error() {
        assert!(parse_args(&a(&["bin", "5", "3"])).is_err());
    }

    #[test]
    fn valid_first_node() {
        assert_eq!(parse_args(&a(&["bin", "0", "1"])).unwrap(), (0, 1));
    }

    #[test]
    fn valid_middle_node() {
        assert_eq!(parse_args(&a(&["bin", "1", "3"])).unwrap(), (1, 3));
    }

    #[test]
    fn valid_last_node() {
        assert_eq!(parse_args(&a(&["bin", "2", "3"])).unwrap(), (2, 3));
    }

    #[test]
    fn valid_large_cluster() {
        assert_eq!(parse_args(&a(&["bin", "7", "10"])).unwrap(), (7, 10));
    }
}
