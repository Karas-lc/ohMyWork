# 21 — 实现独立图片/PDF 预览进程

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` PREVIEW-001、`docs/architecture.md` 第3/8节。确认Pane/Tab可承载只读预览。实现前核验 `pdfium-render 0.9.4`、固定 PDFium `chromium/7881` Windows x64非V8资产、校验和及许可证；不完整则停止打包路径。

预览worker是固定内置解码进程，不是插件宿主或安全沙箱。

## 产物

- `crates/omw-preview-worker/src/{main,protocol,pdf,image,limits}.rs`
- `omw-app/src/ui/preview.rs` 与 `PreviewClient`；页面/缩放/错误UI组合GPUI Kit Toolbar/Button/Progress/Alert等标准组件
- media恶意/损坏夹具、资产manifest

## 进程与协议

- worker单线程串行调用PDFium；Pdfium实例只在worker。应用按需启动单个worker，崩溃/超时可重建。
- 帧：`u32_le header_len + JSON header + payload`；header含version/request_id/kind/payload_len。RGBA走二进制，不用Base64。
- 请求：Open(path)、PageInfo、Render(page,target_size,generation)、Close(document)；回复metadata/bitmap/typed error。
- 所有长度、宽×高×4、stride检查溢出和上限；超限先诊断，不分配。

## 实现顺序

1. 先实现协议/限额/假worker测试，再接真实解码。
2. 从安装目录绝对路径加载 `pdfium.dll`，禁止系统/cwd搜索回退；非V8，只渲染，不执行脚本、外链或文件动作。
3. `image 0.25` 支持PNG/JPEG/WebP/GIF/BMP，GIF仅首帧；预检尺寸。单bitmap初始上限64MiB，超限缩略/分块或明确拒绝。
4. PDF页metadata与按需render；可见页前后各一页预取。缩放先显示旧图，generation新结果到达再替换；过期结果丢弃。
5. bitmap缓存按字节LRU，初始96MiB；Close取消排队render。
6. native调用超时由监督器终止worker并恢复；主UI编辑不受影响。分别报告主进程与worker资源。

## 测试

尺寸炸弹/溢出、畸形图片/PDF、密码/损坏PDF、上百页、快速翻页/缩放取消、worker卡死/崩溃/协议坏帧、DLL劫持路径。确认无脚本/外链自动动作，worker退出后主UI仍可编辑。

## 完成定义

图片/PDF只读且资源有界；其他文件交Windows默认应用。不得把进程隔离宣传为OS权限沙箱。