# 06 — 实现单 Workspace 与 LocalAppData

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` 第2节、`docs/architecture.md` 第1/3节、`docs/adr/adr-0007-single-workspace-storage.md`。确认 app/local crate 可接入当前 workspace。

只实现一个 Workspace 的选择、初始化和全局设置；不做文件扫描、索引、便携迁移或多 Workspace 注册。

## 产物

- `crates/omw-local/src/{lib,paths,workspace,state}.rs`
- `crates/omw-local/tests/{files,portable}.rs` 中本步相关测试
- app 首次设置 UI
- `%LOCALAPPDATA%/ohMyWork/settings.json` 的版本化 schema

## 固定契约

- 唯一 root；`Atlas/`、`Efforts/`、`Archive/` 是真实目录，项目/领域只是普通目录。
- `.ohmywork/id` 是 UTF-8 一行 UUID 加换行；使用 create-new，非法既有内容只诊断，不重置。
- 设置保存使用临时文件+替换；设置与未来可重建索引分开。
- root 路径、Pi启动描述等机器本地数据留在 LocalAppData。

## 实现顺序

1. 定义类型化路径和 settings schema/version，拒绝相对/空/不可访问 root。
2. UI用GPUI Kit Dialog/Form/Button等标准控件组合：选择目录→展示三个分区及将创建的路径→用户确认；系统目录选择仍走安全Windows适配器。
3. 只创建缺失目录和 `.ohmywork/id`；同名文件、只读、部分失败给出可操作错误，不移动已有内容。
4. 安全保存 settings；重启读取并校验。不支持的新 schema 保留原文件并进入诊断流程。
5. 为以后便携模式预留纯数据接口/类型，但不实现复制或双写（留到23）。

## 测试

- 空目录、已有完整目录、部分目录存在、中文/空格/长路径。
- `Atlas` 同名文件、只读目录、并发创建ID、非法ID；均不覆盖内容。
- 重启 root 与 UUID 稳定；设置写失败不破坏旧文件。
- 断言没有项目对象、多 Workspace registry 或自动搬迁笔记。

## 完成定义与交接契约

后续可通过 `WorkspaceState { root, workspace_uuid }` 获取唯一根，并通过路径服务获取 LocalAppData 位置。错误路径不能产生“半完成设置”状态；已存在用户内容必须保持。