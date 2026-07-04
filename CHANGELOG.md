# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-04

### Added

- Initial release: a windswept fire screensaver for egui, ported from
  [Blake Bowen's "Flame in the wind" CodePen](https://codepen.io/osublake/pen/pqNXoq).
- GPU-rendered fractal-noise fragment shader via an `egui::PaintCallback`
  through `egui_glow`, requiring a `glow`-backed `eframe::App`.
- Native and web (WASM) demo examples.

[Unreleased]: https://github.com/cecton/egui-screensaver-flame/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/cecton/egui-screensaver-flame/releases/tag/v0.1.0
