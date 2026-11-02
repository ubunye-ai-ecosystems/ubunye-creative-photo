# Security policy: Ubunye Photo

## Reporting
Please **do not** open public issues for vulnerabilities. Use GitHub's private reporting ("Security" → "Report a vulnerability") on `ubunye-ai-ecosystems/ubunye-creative-photo`. We acknowledge within 5 working days.
If the issue also affects upstream PhotoCraft, we coordinate disclosure with upstream (https://github.com/storytold/photocraft).

## Supported versions
Only the latest `vX.Y.Z-ubunye.N` release. Pre-1.0 builds are previews.

## Threat model summary
- **Untrusted files.** Every importer (PSD/PSB, .pcraft, PDN, Affinity, ORA, TIFF, PNG, JPEG, WebP, HEIF, camera raw, ICC, ABR, SVG, fonts) parses attacker-controlled data with in-house Rust parsers. Fuzzing status: 16 cargo-fuzz targets upstream, run manually only; nightly runs planned in Milestone 2.5.
- **Local automation.** The `--control` port and the MCP server can read and write files. They are off by default; never expose them beyond localhost. Upstream's control channel is token-protected and bound to loopback.
- **Downloads.** Optional AI model downloads are user-initiated and hash-pinned upstream.
- **Supply chain.** `deny.toml` + `cargo audit` in CI (`ubunye-audit.yml`); toolchain pinned; Actions pinned by SHA in Ubunye workflows.

## Known advisories at import (cargo audit, 2026-10-09 DB)
- none (0 vulnerabilities). Warnings: unmaintained `paste`, `ttf-parser`, `rustybuzz`; yanked `yoke-derive` 0.8.3.
