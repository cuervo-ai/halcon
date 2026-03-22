//! # HICON Phase 5: ARIMA Prediction Engine
//!
//! Time series prediction for agent resource optimization using ARIMA (AutoRegressive
//! Integrated Moving Average) models. Predicts round count, token consumption, and cost
//! trajectory to enable proactive resource management.

use std::collections::VecDeque;

/// ARIMA model order parameters.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArimaOrder {
    /// AutoRegressive order (p) — number of lag observations.
    pub p: usize,
    /// Differencing order (d) — degree of differencing for stationarity.
    pub d: usize,
    /// Moving Average order (q) — size of moving average window.
    pub q: usize,
}

impl Default for ArimaOrder {
    fn default() -> Self {
        // ARIMA(1,1,1) — simple but effective for agent metrics
        Self { p: 1, d: 1, q: 1 }
    }
}

/// Time series observation with timestamp.
#[derive(Debug, Clone, Copy)]
struct Observation {
    /// Round number (acts as timestamp).
    round: usize,
    /// Observed value.
    value: f64,
}

/// ARIMA predictor for a single time series.
///
/// Implements a lightweight ARIMA model optimized for agent loop predictions.
/// Uses fixed-order ARIMA(1,1,1) for simplicity and real-time performance.
pub(crate) struct ArimaPredictor {
    /// Model order parameters.
    order: ArimaOrder,

    /// Time series observations (circular buffer).
    observations: VecDeque<Observation>,

    /// Maximum observations to keep (for performance).
    max_observations: usize,

    /// Differenced series (for stationarity).
    differenced: VecDeque<f64>,

    /// AR coefficient (φ).
    ar_coeff: f64,

    /// MA coefficient (θ).
    ma_coeff: f64,

    /// Residuals for MA component.
    residuals: VecDeque<f64>,

    /// Whether model has been fitted.
    fitted: bool,
}

impl ArimaPredictor {
    /// Create new ARIMA predictor with default order (1,1,1).
    pub(crate) fn new() -> Self {
        Self::with_order(ArimaOrder::default())
    }

    /// Create ARIMA predictor with custom order.
    pub(crate) fn with_order(order: ArimaOrder) -> Self {
        Self {
            order,
            observations: VecDeque::new(),
            max_observations: 50, // Keep last 50 rounds
            differenced: VecDeque::new(),
            ar_coeff: 0.5, // Initial estimate
            ma_coeff: 0.5, // Initial estimate
            residuals: VecDeque::new(),
            fitted: false,
        }
    }

    /// Add observation to time series.
    pub(crate) fn add_observation(&mut self, round: usize, value: f64) {
        self.observations.push_back(Observation { round, value });

        // Maintain max size
        if self.observations.len() > self.max_observations {
            self.observations.pop_front();
        }

        // Mark as unfitted (need to refit after new data)
        self.fitted = false;
    }

    /// Fit ARIMA model to current observations.
    ///
    /// Uses simple OLS (Ordinary Least Squares) for coefficient estimation.
    pub(crate) fn fit(&mut self) -> Result<(), String> {
        if self.observations.len() < 5 {
            return Err("Need at least 5 observations to fit ARIMA".to_string());
        }

        // Step 1: Apply differencing (d=1)
        self.apply_differencing();

        if self.differenced.len() < 3 {
            return Err("Insufficient data after differencing".to_string());
        }

        // Step 2: Estimate AR coefficient (φ) via lag-1 autocorrelation
        self.ar_coeff = self.estimate_ar_coefficient();

        // Step 3: Compute AR residuals (ε[t] = y[t] - φ * y[t-1])
        // Must happen BEFORE MA estimation so residuals are available.
        self.compute_residuals();

        // Step 4: Estimate MA coefficient (θ) from residual lag-1 ACF
        self.ma_coeff = self.estimate_ma_coefficient();

        self.fitted = true;
        Ok(())
    }

    /// Predict next value in series.
    ///
    /// Returns predicted value with confidence interval (lower, mean, upper).
    pub(crate) fn predict_next(&self) -> Result<(f64, f64, f64), String> {
        if !self.fitted {
            return Err("Model not fitted yet. Call fit() first.".to_string());
        }

        if self.differenced.is_empty() {
            return Err("No differenced data available".to_string());
        }

        // AR component: φ * y_{t-1}
        let last_diff = self.differenced.back().copied().unwrap_or(0.0);
        let ar_component = self.ar_coeff * last_diff;

        // MA component: θ * ε_{t-1}
        let last_residual = self.residuals.back().copied().unwrap_or(0.0);
        let ma_component = self.ma_coeff * last_residual;

        // Predicted differenced value
        let diff_pred = ar_component + ma_component;

        // Integrate back to original scale (undo differencing)
        let last_value = self.observations.back().map(|obs| obs.value).unwrap_or(0.0);
        let prediction = last_value + diff_pred;

        // Estimate prediction uncertainty (simplified via residual std dev)
        let std_dev = self.residual_std_dev();

        // Ensure std_dev is finite and positive
        let std_dev = if std_dev.is_finite() && std_dev > 0.0 {
            std_dev
        } else {
            1.0 // Default to small uncertainty if calculation fails
        };

        let lower = prediction - 1.96 * std_dev; // 95% CI
        let upper = prediction + 1.96 * std_dev;

        Ok((lower, prediction, upper))
    }

    /// Predict next N values.
    ///
    /// Returns vector of (lower, mean, upper) predictions.
    pub(crate) fn predict_horizon(&self, n: usize) -> Result<Vec<(f64, f64, f64)>, String> {
        if !self.fitted {
            return Err("Model not fitted yet".to_string());
        }

        let mut predictions = Vec::new();
        let mut last_value = self.observations.back().map(|obs| obs.value).unwrap_or(0.0);
        let mut last_diff = self.differenced.back().copied().unwrap_or(0.0);
        let mut last_residual = self.residuals.back().copied().unwrap_or(0.0);

        let std_dev = self.residual_std_dev();
        // Ensure std_dev is finite and positive
        let std_dev = if std_dev.is_finite() && std_dev > 0.0 {
            std_dev
        } else {
            1.0 // Default to small uncertainty if calculation fails
        };

        for _ in 0..n {
            // AR + MA components
            let ar_comp = self.ar_coeff * last_diff;
            let ma_comp = self.ma_coeff * last_residual;
            let diff_pred = ar_comp + ma_comp;

            // Integrate
            let pred = last_value + diff_pred;
            let lower = pred - 1.96 * std_dev;
            let upper = pred + 1.96 * std_dev;

            predictions.push((lower, pred, upper));

            // Update for next iteration
            last_value = pred;
            last_diff = diff_pred;
            last_residual = 0.0; // Future residuals unknown, assume 0
        }

        Ok(predictions)
    }

    /// Check if current observation count is sufficient for fitting.
    pub(crate) fn has_sufficient_data(&self) -> bool {
        self.observations.len() >= 5
    }

    /// Get number of observations.
    pub(crate) fn observation_count(&self) -> usize {
        self.observations.len()
    }

    // === Private Methods ===

    /// Apply differencing to achieve stationarity.
    fn apply_differencing(&mut self) {
        self.differenced.clear();

        if self.observations.len() < 2 {
            return;
        }

        let values: Vec<f64> = self.observations.iter().map(|obs| obs.value).collect();

        // First-order differencing: diff[i] = values[i] - values[i-1]
        for i in 1..values.len() {
            self.differenced.push_back(values[i] - values[i - 1]);
        }
    }

    /// Estimate AR coefficient via lag-1 autocorrelation.
    fn estimate_ar_coefficient(&self) -> f64 {
        if self.differenced.len() < 2 {
            return 0.5; // Default
        }

        let mean = self.differenced.iter().sum::<f64>() / self.differenced.len() as f64;

        let mut numerator = 0.0;
        let mut denominator = 0.0;

        let data: Vec<f64> = self.differenced.iter().copied().collect();

        for i in 0..(data.len() - 1) {
            let dev_t = data[i] - mean;
            let dev_t1 = data[i + 1] - mean;
            numerator += dev_t * dev_t1;
            denominator += dev_t * dev_t;
        }

        if denominator.abs() < 1e-10 {
            return 0.5;
        }

        (numerator / denominator).clamp(-0.99, 0.99) // Stability constraint
    }

    /// Estimate MA(1) coefficient θ via lag-1 autocorrelation of AR residuals.
    ///
    /// Formula: θ ≈ ACF(ε, lag=1) = Σ(ε[t] * ε[t-1]) / Σ(ε[t-1]²)
    ///
    /// Requires `compute_residuals()` to have been called first.
    /// Falls back to 0.3 if residuals are insufficient (< 3 points).
    fn estimate_ma_coefficient(&self) -> f64 {
        let residuals: Vec<f64> = self.residuals.iter().copied().collect();
        if residuals.len() < 3 {
            return 0.3; // conservative fallback when insufficient data
        }

        let mut numerator = 0.0;
        let mut denominator = 0.0;

        for i in 1..residuals.len() {
            numerator += residuals[i] * residuals[i - 1];
            denominator += residuals[i - 1] * residuals[i - 1];
        }

        if denominator.abs() < 1e-10 {
            return 0.3;
        }

        // Clamp to (-0.99, 0.99) for invertibility of the MA process
        (numerator / denominator).clamp(-0.99, 0.99)
    }

    /// Compute residuals for MA component.
    fn compute_residuals(&mut self) {
        self.residuals.clear();

        if self.differenced.len() < 2 {
            return;
        }

        let data: Vec<f64> = self.differenced.iter().copied().collect();

        for i in 1..data.len() {
            let predicted = self.ar_coeff * data[i - 1];
            let residual = data[i] - predicted;
            self.residuals.push_back(residual);
        }
    }

    /// Calculate standard deviation of residuals.
    fn residual_std_dev(&self) -> f64 {
        if self.residuals.is_empty() {
            return 1.0; // Default uncertainty
        }

        let mean = self.residuals.iter().sum::<f64>() / self.residuals.len() as f64;
        let variance = self
            .residuals
            .iter()
            .map(|r| (r - mean).powi(2))
            .sum::<f64>()
            / self.residuals.len() as f64;

        variance.sqrt()
    }
}

/// Resource prediction engine using ARIMA models.
///
/// Maintains separate predictors for rounds, tokens, and cost.
pub(crate) struct ResourcePredictor {
    /// Round count predictor.
    round_predictor: ArimaPredictor,

    /// Input token predictor.
    input_token_predictor: ArimaPredictor,

    /// Output token predictor.
    output_token_predictor: ArimaPredictor,

    /// Cost predictor.
    cost_predictor: ArimaPredictor,

    /// Last fit round (for tracking when to refit).
    last_fit_round: usize,

    /// Refit interval (rounds between refits).
    refit_interval: usize,
}

impl ResourcePredictor {
    /// Create new resource predictor.
    pub(crate) fn new() -> Self {
        Self {
            round_predictor: ArimaPredictor::new(),
            input_token_predictor: ArimaPredictor::new(),
            output_token_predictor: ArimaPredictor::new(),
            cost_predictor: ArimaPredictor::new(),
            last_fit_round: 0,
            refit_interval: 3, // Refit every 3 rounds
        }
    }

    /// Add observation for current round.
    pub(crate) fn observe(
        &mut self,
        round: usize,
        input_tokens: u64,
        output_tokens: u64,
        cost: f64,
    ) {
        self.input_token_predictor
            .add_observation(round, input_tokens as f64);
        self.output_token_predictor
            .add_observation(round, output_tokens as f64);
        self.cost_predictor.add_observation(round, cost);

        // Fit if this is the first time we have sufficient data
        let has_data = self.input_token_predictor.has_sufficient_data();
        let never_fitted = self.last_fit_round == 0;

        if has_data && never_fitted {
            // First fit as soon as we have enough data
            let _ = self.fit_all();
            self.last_fit_round = round;
        } else if round >= self.last_fit_round + self.refit_interval {
            // Refit if interval passed
            let _ = self.fit_all();
            self.last_fit_round = round;
        }
    }

    /// Fit all predictors.
    pub(crate) fn fit_all(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();

        if let Err(e) = self.input_token_predictor.fit() {
            errors.push(format!("Input tokens: {}", e));
        }

        if let Err(e) = self.output_token_predictor.fit() {
            errors.push(format!("Output tokens: {}", e));
        }

        if let Err(e) = self.cost_predictor.fit() {
            errors.push(format!("Cost: {}", e));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Predict resource usage for next N rounds.
    pub(crate) fn predict_resources(&self, horizon: usize) -> ResourcePrediction {
        let input_tokens = self.input_token_predictor.predict_horizon(horizon).ok();
        let output_tokens = self.output_token_predictor.predict_horizon(horizon).ok();
        let cost = self.cost_predictor.predict_horizon(horizon).ok();

        ResourcePrediction {
            horizon,
            input_tokens,
            output_tokens,
            cost,
        }
    }

    /// Check if ready for predictions (sufficient data).
    pub(crate) fn is_ready(&self) -> bool {
        self.input_token_predictor.has_sufficient_data()
            && self.output_token_predictor.has_sufficient_data()
            && self.cost_predictor.has_sufficient_data()
    }
}

/// Predicted resource usage over horizon.
#[derive(Debug, Clone)]
pub(crate) struct ResourcePrediction {
    /// Prediction horizon (number of rounds).
    pub horizon: usize,

    /// Predicted input tokens (lower, mean, upper) per round.
    pub input_tokens: Option<Vec<(f64, f64, f64)>>,

    /// Predicted output tokens (lower, mean, upper) per round.
    pub output_tokens: Option<Vec<(f64, f64, f64)>>,

    /// Predicted cost (lower, mean, upper) per round.
    pub cost: Option<Vec<(f64, f64, f64)>>,
}

impl ResourcePrediction {
    /// Get total predicted tokens (input + output) for horizon.
    pub(crate) fn total_tokens_mean(&self) -> Option<f64> {
        match (&self.input_tokens, &self.output_tokens) {
            (Some(input), Some(output)) => {
                let total: f64 = input
                    .iter()
                    .zip(output.iter())
                    .map(|((_, i_mean, _), (_, o_mean, _))| i_mean + o_mean)
                    .sum();
                Some(total)
            }
            _ => None,
        }
    }

    /// Get total predicted cost for horizon.
    pub(crate) fn total_cost_mean(&self) -> Option<f64> {
        self.cost
            .as_ref()
            .map(|predictions| predictions.iter().map(|(_, mean, _)| mean).sum())
    }

    /// Check if budget will be exceeded within horizon.
    pub(crate) fn exceeds_token_budget(&self, budget: u64) -> bool {
        if let Some(total) = self.total_tokens_mean() {
            total > budget as f64
        } else {
            false
        }
    }

    /// Check if cost budget will be exceeded within horizon.
    pub(crate) fn exceeds_cost_budget(&self, budget: f64) -> bool {
        if let Some(total) = self.total_cost_mean() {
            total > budget
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arima_predictor_creation() {
        let predictor = ArimaPredictor::new();
        assert_eq!(predictor.order.p, 1);
        assert_eq!(predictor.order.d, 1);
        assert_eq!(predictor.order.q, 1);
        assert!(!predictor.fitted);
    }

    #[test]
    fn test_add_observation() {
        let mut predictor = ArimaPredictor::new();
        predictor.add_observation(1, 100.0);
        predictor.add_observation(2, 110.0);
        assert_eq!(predictor.observation_count(), 2);
    }

    #[test]
    fn test_insufficient_data() {
        let mut predictor = ArimaPredictor::new();
        predictor.add_observation(1, 100.0);
        predictor.add_observation(2, 110.0);

        let result = predictor.fit();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least 5 observations"));
    }

    #[test]
    fn test_fit_with_sufficient_data() {
        let mut predictor = ArimaPredictor::new();

        // Add linear growth pattern
        for i in 1..=10 {
            predictor.add_observation(i, i as f64 * 10.0);
        }

        let result = predictor.fit();
        assert!(result.is_ok());
        assert!(predictor.fitted);
    }

    #[test]
    fn test_predict_next() {
        let mut predictor = ArimaPredictor::new();

        // Linear growth: 10, 20, 30, ...
        for i in 1..=10 {
            predictor.add_observation(i, i as f64 * 10.0);
        }

        predictor.fit().unwrap();

        let (lower, mean, upper) = predictor.predict_next().unwrap();
        assert!(lower < mean);
        assert!(mean < upper);
        // Should predict around 110 (next in linear sequence)
        assert!(mean > 90.0 && mean < 130.0);
    }

    #[test]
    fn test_predict_horizon() {
        let mut predictor = ArimaPredictor::new();

        for i in 1..=10 {
            predictor.add_observation(i, i as f64 * 10.0);
        }

        predictor.fit().unwrap();

        let predictions = predictor.predict_horizon(5).unwrap();
        assert_eq!(predictions.len(), 5);

        // Check structure
        for (lower, mean, upper) in &predictions {
            assert!(lower < mean);
            assert!(mean < upper);
        }
    }

    #[test]
    fn test_resource_predictor() {
        let mut predictor = ResourcePredictor::new();

        // Add observations
        for round in 1..=10 {
            predictor.observe(
                round,
                1000 + round as u64 * 100,
                500 + round as u64 * 50,
                0.01 * round as f64,
            );
        }

        assert!(predictor.is_ready());

        // Ensure models are fitted
        let _ = predictor.fit_all();

        let prediction = predictor.predict_resources(3);
        assert_eq!(prediction.horizon, 3);
        assert!(prediction.input_tokens.is_some());
        assert!(prediction.output_tokens.is_some());
        assert!(prediction.cost.is_some());
    }

    #[test]
    fn test_total_tokens_mean() {
        let prediction = ResourcePrediction {
            horizon: 2,
            input_tokens: Some(vec![(90.0, 100.0, 110.0), (190.0, 200.0, 210.0)]),
            output_tokens: Some(vec![(40.0, 50.0, 60.0), (90.0, 100.0, 110.0)]),
            cost: None,
        };

        let total = prediction.total_tokens_mean().unwrap();
        assert_eq!(total, 450.0); // (100 + 50) + (200 + 100)
    }

    #[test]
    fn test_exceeds_token_budget() {
        let prediction = ResourcePrediction {
            horizon: 2,
            input_tokens: Some(vec![(90.0, 100.0, 110.0), (190.0, 200.0, 210.0)]),
            output_tokens: Some(vec![(40.0, 50.0, 60.0), (90.0, 100.0, 110.0)]),
            cost: None,
        };

        assert!(prediction.exceeds_token_budget(400)); // 450 > 400
        assert!(!prediction.exceeds_token_budget(500)); // 450 < 500
    }

    #[test]
    fn test_exceeds_cost_budget() {
        let prediction = ResourcePrediction {
            horizon: 3,
            input_tokens: None,
            output_tokens: None,
            cost: Some(vec![
                (0.01, 0.02, 0.03),
                (0.02, 0.03, 0.04),
                (0.03, 0.04, 0.05),
            ]),
        };

        let total = prediction.total_cost_mean().unwrap();
        assert_eq!(total, 0.09); // 0.02 + 0.03 + 0.04

        assert!(prediction.exceeds_cost_budget(0.05)); // 0.09 > 0.05
        assert!(!prediction.exceeds_cost_budget(0.10)); // 0.09 < 0.10
    }

    #[test]
    fn test_differencing() {
        let mut predictor = ArimaPredictor::new();

        predictor.add_observation(1, 10.0);
        predictor.add_observation(2, 15.0);
        predictor.add_observation(3, 18.0);
        predictor.add_observation(4, 23.0);
        predictor.add_observation(5, 25.0);

        predictor.apply_differencing();

        // Differences: 5, 3, 5, 2
        assert_eq!(predictor.differenced.len(), 4);
        assert_eq!(predictor.differenced[0], 5.0);
        assert_eq!(predictor.differenced[1], 3.0);
        assert_eq!(predictor.differenced[2], 5.0);
        assert_eq!(predictor.differenced[3], 2.0);
    }

    #[test]
    fn test_ar_coefficient_estimation() {
        let mut predictor = ArimaPredictor::new();

        // Add stationary-ish data
        for i in 1..=10 {
            let value = 50.0 + (i as f64 * 0.5).sin() * 10.0;
            predictor.add_observation(i, value);
        }

        predictor.apply_differencing();
        let ar_coeff = predictor.estimate_ar_coefficient();

        // Should be in stable range
        assert!(ar_coeff > -0.99 && ar_coeff < 0.99);
    }
}
