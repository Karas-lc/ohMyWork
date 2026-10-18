Plan 27: Windows per-user NSIS installer + manual upgrade (current).
- Location: packaging/windows/{installer.nsi, assets-manifest.json, THIRD-PARTY-NOTICES.txt}
- Per user install to %LOCALAPPDATA%\Programs\ohMyWork (no admin, no elevation).
- Includes main (ohmywork.exe), omw-preview-worker.exe + pinned pdfium.dll (chromium/7881), resources, licenses.
- No Pi download, no update service, fully offline.
- Manual upgrade: manifest/version verify then replace + state forward; rollback on fail.
- Uninstall: keep Workspace + LocalAppData by default (explicit unchecked option to clear appdata); NEVER delete Workspace.
- See .agents/plans/ohmywork-v1/27-installer.md , docs/product-requirements.md DIST-001, docs/architecture.md §9.
- smoke-install.ps1 and release.yml implement the checks + pinned --locked build.
---
OLD STUB (historical):
Plan 27: Windows installer (Inno or wix skeleton).
- Per user install (no admin).
- Portable toggle copies to .ohmywork.
- No elevation for normal.
- See AGENTS and docs/install.md for full.
