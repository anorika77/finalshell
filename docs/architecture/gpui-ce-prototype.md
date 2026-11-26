# GPUI CE prototype — validation notes

> Status: initial cross-platform smoke prototype; not an architecture decision.

## Version pinning

- `gpui-ce = 0.3.0`
- `gpui_ce_platform = 0.3.0`
- Rust MSRV: 1.95 (the repository toolchain is newer).
- Both crates are pinned exactly to avoid silently following a moving pre-1.0 API.

## Prototype scope

The initial window establishes the same broad workspace composition used by the Iced spike:

- top-level application toolbar;
- left connection area (1,000-item fixture contract);
- center terminal workspace with four pane/session identity pairs;
- right monitor area;
- bottom local-only quick-command placeholder;
- 10,000-item file fixture contract.

No SSH, PTY, remote filesystem, persistence, or cloud sync is connected.

## Current limitations

This first commit is a build/API compatibility gate, not a feature parity claim. The lists are represented by fixture counts rather than virtualized interactive widgets; pane controls are labels rather than working controls. Follow-up steps will add interaction and virtualization only after the five-platform dependency/build gate is green.

GPUI's headless `TestApp`/ `#[gpui::test]` support is available behind the `test-support` feature, but it is not yet used here. The first tests intentionally isolate product invariants from renderer setup.

## Platform requirements

Linux CI installs the native X11/Wayland/font development libraries required by GPUI CE. Real desktop IME, GPU driver behavior, candidate-window placement, DPI, and accessibility still require desktop smoke testing.

## References

- https://docs.rs/crate/gpui-ce/0.3.0
- https://docs.rs/crate/gpui_ce_platform/0.3.0
- https://github.com/gpui-ce/gpui-ce/blob/main/CHANGELOG.md
