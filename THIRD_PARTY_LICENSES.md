# Third-party licenses

This is the initial license inventory for the V1 bootstrap. `Cargo.lock` is the authoritative
version inventory. Before distribution, this file must be regenerated from the locked dependency
graph and bundled with the complete license and notice texts required by every dependency.

## Direct UI dependency

| Package | Locked version | License | Source |
|---|---:|---|---|
| `gpui-kit` | 0.7.0 | Apache-2.0 | <https://github.com/longbridge/gpui-kit> |

GPUI Kit's default features also resolve its matching first-party packages, including
`gpui-base`, `gpui-component`, and `gpui-kit-assets` 0.7.0 (Apache-2.0). The GPUI implementation
is consumed only through GPUI Kit and resolves the `gpui-pre` family at 0.3.7 (Apache-2.0).
`gpui-pre-reqwest` retains reqwest's 0.12.15 version and is offered under MIT OR Apache-2.0.

## Documentation and example provenance

The bootstrap follows the public GPUI Kit Getting Started, testing, and accessibility API
patterns. GPUI Kit software and code examples are licensed under Apache-2.0. Its original
documentation text and diagrams are separately available under CC BY 4.0; this repository does
not copy that prose or those diagrams.

- <https://gpui-kit.com/zh-CN/docs/getting-started>
- <https://gpui-kit.com/zh-CN/docs/test>
- <https://gpui-kit.com/zh-CN/docs/accessibility>

## Release follow-up

- Enumerate the target-specific production graph from the committed lockfile.
- Preserve each package's selected license expression and required notices.
- Bundle full license texts with installer artifacts.
- Review icon, font, and other asset licenses independently from Rust crate metadata.
