//! # Momoto Metrics
//!
//! Contrast metric implementations (APCA, WCAG 2.x).
//!
//! This crate provides concrete implementations of the `ContrastMetric` trait
//! from `momoto-core`.
//!
//! ## Features
//!
//! - `internals`: Exposes algorithm constants for debugging and education
//!
//! ## Quick Start
//!
//! ```rust
//! use momoto_core::color::Color;
//! use momoto_core::perception::ContrastMetric;
//! use momoto_metrics::{APCAMetric, WCAGMetric};
//!
//! let black = Color::from_srgb8(0, 0, 0);
//! let white = Color::from_srgb8(255, 255, 255);
//!
//! // APCA contrast (signed Lc value)
//! let apca = APCAMetric.evaluate(black, white);
//! assert!((apca.value - 106.04).abs() < 0.1);
//!
//! // WCAG contrast ratio
//! let wcag = WCAGMetric.evaluate(black, white);
//! assert!((wcag.value - 21.0).abs() < 0.01);
//! ```

// Note: no_std support planned for future release
#![warn(
    missing_docs,
    missing_debug_implementations,
    rust_2018_idioms,
    unreachable_pub
)]

pub mod apca;
pub mod wcag;

// TODO: SAPC (Simplified APCA) implementation
// Planned for future release
// pub mod sapc;

// Re-export metrics
pub use apca::APCAMetric;
pub use wcag::{TextSize, WCAGLevel, WCAGMetric, WCAG_REQUIREMENTS};

// Re-export APCA constants when internals feature is enabled
#[cfg(feature = "internals")]
pub use apca::constants as apca_constants;
