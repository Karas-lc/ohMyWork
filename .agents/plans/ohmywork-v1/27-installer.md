# 27 — 制作当前用户安装器与手动升级

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` DIST-001、`docs/architecture.md` 第9节和所有第三方依赖许可证。确认release locked构建、preview资产校验、状态migration和性能验证可运行。

本步制作Windows 11 x64当前用户NSIS安装/升级/卸载；不做自动更新、不下载Pi、不要求管理员权限。

## 产物

- `packaging/windows/{installer.nsi,assets-manifest.json,THIRD-PARTY-NOTICES.txt}`
- `tools/smoke-install.ps1`
- `.github/workflows/release.yml`
- 可复核的产物校验和与版本元数据

## 安装契约

- 安装到用户LocalAppData下Programs/ohMyWork；包含主程序、preview worker、固定 `pdfium.dll`、必要资源和许可证。GPUI Kit默认图标等编译期资产应嵌入主程序，不从cwd加载。
- `fake_pi`、`perf_runner`、测试fixture不进入安装包。
- 创建当前用户快捷方式和卸载注册，不写机器级目录/注册表，不提升权限。
- 安装完全离线；不安装/更新/配置Pi，不创建更新服务。

## 升级与卸载

1. 手动运行新版安装器；先验证manifest/版本，再替换程序资产，执行向前状态migration。
2. 升级失败可回退程序和迁移前持久设置备份；索引可重建但不能删布局/主题冒充成功。
3. 卸载默认保留Workspace和LocalAppData。清除应用数据是独立未默认勾选的明确选择；无论如何绝不删除Workspace。
4. 运行中的文件、preview worker、路径空格/中文和重启需求有清晰处理。
5. 只使用实际证书签名；无证书的测试包明确标“未签名”，不承诺绕过SmartScreen。

## 测试

Windows 11标准用户：干净安装、启动/预览、Kit默认图标/主题/对话框/Dock可用、路径空格/中文、旧版→新版、故障注入回滚、卸载默认保留、选择清数据、卸载重装。检查无管理员提示、无网络下载Pi/更新服务、GPUI Kit/gpui-pre/资源及PDFium的NOTICE/hash完整、安装包不含测试binary。

`smoke-install.ps1` 对可自动项给出非零失败；人工项单列。release workflow固定工具链/runner并使用`--locked`。

## 完成定义

可在无管理员权限的目标机安装、手动升级和卸载，用户Workspace始终不受卸载影响。许可证或PDFium资产来源/校验不完整时不得产出候选发布包。