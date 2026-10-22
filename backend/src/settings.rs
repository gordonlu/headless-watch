use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub interval_s: f64,
    pub cpu_alert_cores: f64,
    pub cpu_alert_duration_s: f64,
    pub leftover_after_s: f64,
    pub pss_interval_s: f64,
    pub cdp_interval_s: f64,
    pub notifications: bool,
    pub global_mute: bool,
    pub allow_screenshots: bool,
    pub history_max: usize,
    pub retention_raw_hours: u32,
    pub retention_events_days: u32,
    pub cpu_clear_ratio: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            interval_s: 1.0,
            cpu_alert_cores: 1.0,
            cpu_alert_duration_s: 30.0,
            leftover_after_s: 60.0,
            pss_interval_s: 15.0,
            cdp_interval_s: 5.0,
            notifications: true,
            global_mute: false,
            allow_screenshots: true,
            history_max: 360,
            retention_raw_hours: 24,
            retention_events_days: 30,
            cpu_clear_ratio: 0.75,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.5..=10.0).contains(&self.interval_s) {
            return Err("interval_s must be between 0.5 and 10".to_string());
        }
        if !(0.05..=1024.0).contains(&self.cpu_alert_cores) {
            return Err("cpu_alert_cores must be between 0.05 and 1024".to_string());
        }
        if !(5.0..=3600.0).contains(&self.cpu_alert_duration_s) {
            return Err("cpu_alert_duration_s must be between 5 and 3600".to_string());
        }
        if !(10.0..=86400.0).contains(&self.leftover_after_s) {
            return Err("leftover_after_s must be between 10 and 86400".to_string());
        }
        if !(5.0..=300.0).contains(&self.pss_interval_s) {
            return Err("pss_interval_s must be between 5 and 300".to_string());
        }
        if !(2.0..=120.0).contains(&self.cdp_interval_s) {
            return Err("cdp_interval_s must be between 2 and 120".to_string());
        }
        if !(30..=3600).contains(&self.history_max) {
            return Err("history_max must be between 30 and 3600".to_string());
        }
        if !(0.5..=0.99).contains(&self.cpu_clear_ratio) {
            return Err("cpu_clear_ratio must be between 0.5 and 0.99".to_string());
        }
        if self.retention_raw_hours == 0 || self.retention_events_days == 0 {
            return Err("retention values must be greater than zero".to_string());
        }
        Ok(())
    }

    pub fn normalized(mut self) -> Self {
        self.interval_s = self.interval_s.clamp(0.5, 10.0);
        self.cpu_alert_cores = self.cpu_alert_cores.clamp(0.05, 1024.0);
        self.cpu_alert_duration_s = self.cpu_alert_duration_s.clamp(5.0, 3600.0);
        self.leftover_after_s = self.leftover_after_s.clamp(10.0, 86400.0);
        self.pss_interval_s = self.pss_interval_s.clamp(5.0, 300.0);
        self.cdp_interval_s = self.cdp_interval_s.clamp(2.0, 120.0);
        self.history_max = self.history_max.clamp(30, 3600);
        self.retention_raw_hours = self.retention_raw_hours.max(1);
        self.retention_events_days = self.retention_events_days.max(1);
        self.cpu_clear_ratio = self.cpu_clear_ratio.clamp(0.5, 0.99);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_ranges() {
        let mut settings = Settings::default();
        settings.interval_s = 0.1;
        assert!(settings.validate().is_err());
        settings.interval_s = 1.0;
        settings.pss_interval_s = 1.0;
        assert!(settings.validate().is_err());
    }

    #[test]
    fn defaults_are_valid() {
        assert!(Settings::default().validate().is_ok());
    }
}
