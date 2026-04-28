//! Tests for proactive disk scaling

#[cfg(test)]
mod tests {
    use super::super::disk_scaler::*;

    #[test]
    fn test_calculate_percent() {
        assert_eq!(DiskUsage::calculate_percent(50, 100), 50);
        assert_eq!(DiskUsage::calculate_percent(80, 100), 80);
        assert_eq!(DiskUsage::calculate_percent(100, 100), 100);
        assert_eq!(DiskUsage::calculate_percent(0, 100), 0);
        assert_eq!(DiskUsage::calculate_percent(50, 0), 0);
        assert_eq!(DiskUsage::calculate_percent(150, 100), 100); // Capped at 100
    }

    #[test]
    fn test_parse_quantity_to_bytes() {
        assert_eq!(
            parse_quantity_to_bytes("100Gi").unwrap(),
            100 * 1024 * 1024 * 1024
        );
        assert_eq!(
            parse_quantity_to_bytes("1Ti").unwrap(),
            1024 * 1024 * 1024 * 1024
        );
        assert_eq!(parse_quantity_to_bytes("500Mi").unwrap(), 500 * 1024 * 1024);
        assert_eq!(parse_quantity_to_bytes("1Gi").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(
            parse_quantity_to_bytes("100G").unwrap(),
            100 * 1000 * 1000 * 1000
        );
    }

    #[test]
    fn test_format_bytes_to_quantity() {
        assert_eq!(format_bytes_to_quantity(100 * 1024 * 1024 * 1024), "100Gi");
        assert_eq!(format_bytes_to_quantity(1024 * 1024 * 1024 * 1024), "1Ti");
        assert_eq!(
            format_bytes_to_quantity(1536 * 1024 * 1024 * 1024),
            "1536Gi"
        );

        // Test rounding up
        assert_eq!(
            format_bytes_to_quantity(100 * 1024 * 1024 * 1024 + 1),
            "101Gi"
        );
    }

    #[test]
    fn test_calculate_new_size() {
        assert_eq!(calculate_new_size("100Gi", 50).unwrap(), "150Gi");
        assert_eq!(calculate_new_size("1Ti", 50).unwrap(), "1536Gi");
        assert_eq!(calculate_new_size("200Gi", 25).unwrap(), "250Gi");
        assert_eq!(calculate_new_size("500Gi", 100).unwrap(), "1000Gi");
    }

    #[test]
    fn test_parse_df_output() {
        let output = "Filesystem     1B-blocks      Used Available Use% Mounted on\n\
                      /dev/xvda1   1610612736000 644245094400 966367641600  40% /data";

        let usage = parse_df_output(output).unwrap().unwrap();
        assert_eq!(usage.capacity_bytes, 1610612736000);
        assert_eq!(usage.used_bytes, 644245094400);
        assert_eq!(usage.usage_percent, 40);
    }

    #[test]
    fn test_parse_df_output_high_usage() {
        let output = "Filesystem     1B-blocks      Used Available Use% Mounted on\n\
                      /dev/xvda1   1000000000000 850000000000 150000000000  85% /data";

        let usage = parse_df_output(output).unwrap().unwrap();
        assert_eq!(usage.capacity_bytes, 1000000000000);
        assert_eq!(usage.used_bytes, 850000000000);
        assert_eq!(usage.usage_percent, 85);
    }

    #[test]
    fn test_parse_df_output_no_data_mount() {
        let output = "Filesystem     1B-blocks      Used Available Use% Mounted on\n\
                      /dev/xvda1   1000000000000 500000000000 500000000000  50% /";

        let usage = parse_df_output(output).unwrap();
        assert!(usage.is_none()); // No /data mount found
    }

    #[test]
    fn test_disk_scaler_config_defaults() {
        let config = DiskScalerConfig::default();
        assert_eq!(config.expansion_threshold, 80);
        assert_eq!(config.expansion_increment, 50);
        assert_eq!(config.min_expansion_interval_secs, 3600);
        assert_eq!(config.max_expansions, 10);
        assert!(config.enabled);
    }

    #[test]
    fn test_disk_usage_calculation() {
        let usage = DiskUsage {
            capacity_bytes: 1000,
            used_bytes: 800,
            usage_percent: DiskUsage::calculate_percent(800, 1000),
        };
        assert_eq!(usage.usage_percent, 80);
    }

    #[test]
    fn test_expansion_size_calculation() {
        // Test 50% increment
        let new_size = calculate_new_size("100Gi", 50).unwrap();
        assert_eq!(new_size, "150Gi");

        // Test 100% increment (double)
        let new_size = calculate_new_size("100Gi", 100).unwrap();
        assert_eq!(new_size, "200Gi");

        // Test 25% increment
        let new_size = calculate_new_size("100Gi", 25).unwrap();
        assert_eq!(new_size, "125Gi");
    }

    #[test]
    fn test_quantity_parsing_edge_cases() {
        // Test various units
        assert!(parse_quantity_to_bytes("100Ki").is_ok());
        assert!(parse_quantity_to_bytes("100Mi").is_ok());
        assert!(parse_quantity_to_bytes("100Gi").is_ok());
        assert!(parse_quantity_to_bytes("100Ti").is_ok());
        assert!(parse_quantity_to_bytes("100k").is_ok());
        assert!(parse_quantity_to_bytes("100M").is_ok());
        assert!(parse_quantity_to_bytes("100G").is_ok());
        assert!(parse_quantity_to_bytes("100T").is_ok());

        // Test invalid formats
        assert!(parse_quantity_to_bytes("invalid").is_err());
        assert!(parse_quantity_to_bytes("100Xi").is_err());
    }

    #[test]
    fn test_multiple_expansions() {
        // Simulate multiple expansions
        let mut size = "100Gi".to_string();

        // First expansion: 100Gi -> 150Gi
        size = calculate_new_size(&size, 50).unwrap();
        assert_eq!(size, "150Gi");

        // Second expansion: 150Gi -> 225Gi
        size = calculate_new_size(&size, 50).unwrap();
        assert_eq!(size, "225Gi");

        // Third expansion: 225Gi -> 337Gi (rounded)
        size = calculate_new_size(&size, 50).unwrap();
        assert_eq!(size, "338Gi"); // Rounded up from 337.5
    }
}
