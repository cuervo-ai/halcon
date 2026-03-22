// momoto-core/src/color/operations.rs
//
// Color manipulation operations in perceptually uniform OKLCH space.
//
// These operations are exposed to WASM and consumed by momoto-ui.

use crate::color::Color;

impl Color {
    /// Lighten color by amount in perceptually uniform OKLCH space.
    ///
    /// # Arguments
    /// * `amount` - Lightness increase (0.0 to 1.0)
    ///
    /// # Returns
    /// New Color with increased lightness
    ///
    /// # Example
    /// ```
    /// use momoto_core::color::Color;
    /// let color = Color::from_hex("#3B82F6").unwrap();
    /// let lighter = color.lighten(0.1); // 10% lighter
    /// ```
    pub fn lighten(&self, amount: f64) -> Color {
        let oklch = self.to_oklch();
        Color::from_oklch((oklch.l + amount).min(1.0), oklch.c, oklch.h)
    }

    /// Darken color by amount in perceptually uniform OKLCH space.
    ///
    /// # Arguments
    /// * `amount` - Lightness decrease (0.0 to 1.0)
    ///
    /// # Returns
    /// New Color with decreased lightness
    ///
    /// # Example
    /// ```
    /// use momoto_core::color::Color;
    /// let color = Color::from_hex("#3B82F6").unwrap();
    /// let darker = color.darken(0.1); // 10% darker
    /// ```
    pub fn darken(&self, amount: f64) -> Color {
        let oklch = self.to_oklch();
        Color::from_oklch((oklch.l - amount).max(0.0), oklch.c, oklch.h)
    }

    /// Increase saturation (chroma) by amount.
    ///
    /// # Arguments
    /// * `amount` - Chroma increase
    ///
    /// # Returns
    /// New Color with increased chroma
    ///
    /// # Example
    /// ```
    /// use momoto_core::color::Color;
    /// let color = Color::from_hex("#808080").unwrap();
    /// let vibrant = color.saturate(0.15); // More saturated
    /// ```
    pub fn saturate(&self, amount: f64) -> Color {
        let oklch = self.to_oklch();
        Color::from_oklch(oklch.l, oklch.c + amount, oklch.h)
    }

    /// Decrease saturation (chroma) by amount.
    ///
    /// # Arguments
    /// * `amount` - Chroma decrease
    ///
    /// # Returns
    /// New Color with decreased chroma
    ///
    /// # Example
    /// ```
    /// use momoto_core::color::Color;
    /// let color = Color::from_hex("#FF0000").unwrap();
    /// let muted = color.desaturate(0.1); // Less saturated
    /// ```
    pub fn desaturate(&self, amount: f64) -> Color {
        let oklch = self.to_oklch();
        Color::from_oklch(oklch.l, (oklch.c - amount).max(0.0), oklch.h)
    }

    /// Set alpha channel value.
    ///
    /// # Arguments
    /// * `alpha` - Alpha value (0.0 = transparent, 1.0 = opaque)
    ///
    /// # Returns
    /// New Color with specified alpha
    ///
    /// # Example
    /// ```
    /// use momoto_core::color::Color;
    /// let color = Color::from_hex("#3B82F6").unwrap();
    /// let semi_transparent = color.with_alpha(0.5); // 50% opacity
    /// ```
    pub fn with_alpha(&self, alpha: f64) -> Color {
        let mut color = *self;
        color.alpha = alpha.clamp(0.0, 1.0);
        color
    }

    /// Get alpha channel value.
    ///
    /// # Returns
    /// Alpha value (0.0 to 1.0)
    pub fn get_alpha(&self) -> f64 {
        self.alpha
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lighten() {
        let color = Color::from_hex("#808080").unwrap();
        let lighter = color.lighten(0.1);
        let oklch = lighter.to_oklch();

        // Lightness should increase
        assert!(oklch.l > color.to_oklch().l);
        assert!(oklch.l <= 1.0);
    }

    #[test]
    fn test_darken() {
        let color = Color::from_hex("#808080").unwrap();
        let darker = color.darken(0.1);
        let oklch = darker.to_oklch();

        // Lightness should decrease
        assert!(oklch.l < color.to_oklch().l);
        assert!(oklch.l >= 0.0);
    }

    #[test]
    fn test_saturate() {
        let color = Color::from_hex("#808080").unwrap();
        let vibrant = color.saturate(0.1);
        let oklch = vibrant.to_oklch();

        // Chroma should increase
        assert!(oklch.c > color.to_oklch().c);
    }

    #[test]
    fn test_desaturate() {
        let color = Color::from_hex("#FF0000").unwrap();
        let muted = color.desaturate(0.1);
        let oklch = muted.to_oklch();

        // Chroma should decrease
        assert!(oklch.c < color.to_oklch().c);
        assert!(oklch.c >= 0.0);
    }

    #[test]
    fn test_with_alpha() {
        let color = Color::from_hex("#3B82F6").unwrap();
        let transparent = color.with_alpha(0.5);

        assert_eq!(transparent.get_alpha(), 0.5);
    }

    #[test]
    fn test_alpha_clamping() {
        let color = Color::from_hex("#3B82F6").unwrap();

        // Test clamping at upper bound
        let too_high = color.with_alpha(1.5);
        assert_eq!(too_high.get_alpha(), 1.0);

        // Test clamping at lower bound
        let too_low = color.with_alpha(-0.5);
        assert_eq!(too_low.get_alpha(), 0.0);
    }
}
