# V1 修复验证记录

本记录替代原 Plan 28 的“全部已验证”结论。计划完成、模块测试通过、真实工作台可用和发布验收通过是不同状态；不得互相代替。

环境：2026-10-18，Windows x64，Rust 1.98.1，MSVC target，GPUI Kit 0.7.0 / gpui-pre 0.3.7。仅记录本机实际执行，不代表 CI 或发布认证。

## 滚动卡顿与多文件崩溃补充验证

针对单文件滚轮卡顿、多文件打开后退出，另外定位并修复了两条路径：

- Comrak 的列表容器可能返回结束列为 `0` 的源位置。旧转换让子范围超出父范围，构造尾部 gap 时触发 `half-open range must be ordered`。现在自底向上用直接子节点的完整范围确定零列终点，保留边界空白在正确的父 gap 中；未删除断言或吞掉 panic。
- 编辑器每次滚动都重建全文投影、逐块重新扫描源边界，并重复计算全文 UTF-16 偏移。现在按共享 Rope 实例缓存投影、块边界与字符偏移，使用有序源边界查找；实际修改和外部替换会失效缓存并刷新列表高度。未改变的磁盘轮询结果不再通知文档或触发工作台重绘。

最小崩溃输入 `- [ \n\n#` 已保留为永久回归。只读扫描报告问题的 Workspace 中 104 个 Markdown 文件，全部通过解析、源范围验证及逐字节序列化检查；未改写原文件。

相同 debug 原生窗口、30 段中文/英文/emoji 文本、每组 12 个滚轮样本：

| 场景 | 修复前 P50 | 修复后 P50 | 修复前 P95 | 修复后 P95 |
| --- | ---: | ---: | ---: | ---: |
| 单文档 | 1505.889 ms | 29.367 ms | 2166.149 ms | 45.337 ms |
| 双文档并排 | 3889.300 ms | 70.057 ms | 4341.927 ms | 89.980 ms |

这里测量的是测试 helper 的事件及同步帧调用总耗时，不是单帧 GPU 时间、真实滚轮到像素延迟或 release 性能预算。另以实际文件在真实 Shell/Dock 中依次打开 3 个标签并逐个滚动，检查内存内容与磁盘原文一致、执行到 `WORKBENCH_SCROLL_COMPLETE`，并检查目标窗口截图；未出现原范围 panic 或退出。该工作台场景的 helper P95 为 93.440 / 143.532 / 100.674 ms，不据此声称 GUI 预算达标。

## 本轮修复与原生验证

使用隔离的 Workspace、LocalAppData、Markdown、PNG、两页 PDF 和独立 RPC fixture 进程。临时原生驱动打开真实 GPUI 窗口，通过窗口事件执行交互，而不是只调用 reducer；已检查目标窗口截图。驱动及 fixture 不作为产品功能保留，不修改用户的 Pi 或全局配置。

| 缺陷与契约 | 实际验证 |
| --- | --- |
| 首次设置后仍停留在设置页 | `bootstrap_ui` 检查初始化后进入工作台；原生工作台展示真实目录树和 Dock |
| Markdown 面板只有文件名 | 从文件树打开磁盘文件，在真实编辑器输入；保存前磁盘字节不变 |
| 正文保存和关闭为空操作 | Ctrl+S 写入实际文件；Ctrl+W 后分别执行保存、放弃、取消，核对磁盘与面板状态；实际 Windows `WM_CLOSE` 验证取消保留窗口、保存先落盘再异步关闭 |
| 外部优先 | 脏正文存在时外部改写文件；工作副本替换为外部内容，撤销历史清空 |
| 任务字段插入位置错误 | `task_commit` 的缺失字段回归覆盖 BOM、CRLF、中文、多任务和必要范围写回 |
| 任务提交误保存正文 / 撤销回退已提交字段 | 原生任务组件改 priority，仅任务范围落盘；脏正文保留；撤销正文仍保留已提交 priority |
| 旧任务行修改了外部替换后的任务 | 原生探针先复现错误写盘，再验证任务行磁盘版本与打开文档基线同时匹配；不匹配时拒绝写入，替换任务保持原样 |
| Pi 使用假 cwd、草稿和附件 | 原生面板读取实际输入；独立 fixture 记录选定 cwd、中文草稿、匹配 ID 的 Extension confirm 回复，以及关闭时的 abort。文件附件由用户通过原生选择器显式选择，不使用临时 demo 路径 |
| 预览用颜色块代替图片 | 真实 worker 返回位图，原生面板显示 PNG；使用真实 PDFium 解码两页 PDF，切页和缩放后显示新帧 |
| Homepage 用假位置 / 占位矩形 | 原生添加组件、适合全部、实际指针拖动、键盘移动及调整尺寸、缩放；核对持久化状态；分区入口展开真实目录 |
| Dock 恢复使用固定示例路径 | 新建工作台恢复真实文件标签；磁盘内容重新载入，不恢复未保存正文 |
| 搜索没有打开文档的闭环 | 在真实搜索输入框查询，点击结果打开对应磁盘 Markdown 文档 |

原生驱动执行到 `NATIVE_SMOKE_COMPLETE` 才算完成，不能仅因进程退出码为零而判通过。Pi 上述验证使用 RPC fixture，不声称真实提供商、模型或用户 Extension 全部兼容。原生文件选择器的人工选取流程未自动化。

## 自动回归与构建

```powershell
cargo test --workspace --locked
cargo build --locked -p omw-app -p omw-preview-worker --bins
```

最新全量测试通过：268 tests。生产二进制构建通过，仍有未清理的编译警告。此前实际执行 `cargo clippy --workspace --locked --all-targets -- -D warnings` 未通过，报告包括 `task.rs` 的 `collapsible_if`、Pi `codec.rs` 的 `byte_char_slices` 及 `wire.rs` 等 lint；未放宽规则，本次未声称严格 Clippy 零警告。

关键永久回归位于：

- `crates/omw-document/tests/task_commit.rs`：缺失任务元数据的源范围、局部提交跨撤销/重做。
- `crates/omw-app/tests/bootstrap_ui.rs`：初始化后进入工作台，而不是只断言状态文案。
- `crates/omw-local/src/index/normalize.rs`：规范化查询映射到源字符范围，不多带前一个字符。
- `crates/omw-document/tests/range_regression.rs`：零列列表容器的有序、无损源范围。
- `crates/omw-app/tests/editor_surface.rs`：滚动和本地编辑后收到外部替换，显示新正文，后续输入与撤销不使用旧缓存。
- 现有编辑、Raw、Unicode、版本化写盘、ignore、索引、Pi framing/UI request、预览资源限制及状态测试随全量命令运行。

删除了只固定内部 gram 编码字符串的断言及包含恒真断言的重复主题导入测试；保留面向搜索结果、源边界与主题行为的验证。测试通过不等于下列硬件和发布项通过。

## 性能证据边界

旧 runner 把 `IndexStore::stats()` 当热启动、字符串格式化当输入布局、runner 自身内存当十标签工作台内存。这些数字作废，不再与产品预算比较。

当前 `tools/bench.ps1` 构建 locked release runner，只测真实索引和搜索 API，不添加伪造的 debounce 等待。原始样本及精确测量边界见 `tools/perf-results-latest.txt`；报告格式见 `tools/perf-report-template.md`。

已执行小语料 release 测量：语料 manifest 为 200 notes / 624 tasks，索引报告 201 files / 312 tasks。Archive、ignore 和文件类型会影响计数；该差异没有被当作正确性或验收通过证明。

| 后端指标 | 实测 |
| --- | ---: |
| 首次索引 | 5.6001957 s |
| 搜索 API P50 | 441.5701 ms |
| 搜索 API P95 | 476.4562 ms |

搜索样本不包含 UI debounce、调度、绘制或显示。启动到可交互、输入到像素、十标签主进程内存、GPU、worker/Pi 资源及完整大语料 P95 均未测，不宣称任何 GUI 性能预算达标。

## 发布前仍须验证

- 真实中文输入法组合输入、Narrator、键盘可访问性全覆盖和不同 Windows 缩放比例。
- 用户安装的官方 Pi、真实模型、会话恢复、各种用户 Extension 的端到端兼容；本轮不安装、升级或修改 Pi。
- 原生文件选择器的人工交互，以及预览进程异常退出后的人工恢复场景。
- NSIS 候选包、安装 / 手动升级 / 卸载、签名、中文及空格路径、占用文件、Workspace 保留。
- 完整语料下的 GUI 启动、搜索首批、输入到显示、十标签内存和 GPU 预算。
- 严格 Clippy、仓库级格式检查与 CI。修复模块已使用 rustfmt，不能据此推断整个仓库格式干净。

PDFium 仅为本机验证放在忽略的构建输出目录，未作为源码依赖提交。发布包必须按 `packaging/windows/assets-manifest.json` 提供对应二进制及校验，不能把本机临时 DLL 当作分发证明。

当前结论：本轮审计缺陷的实际交互闭环已有原生证据；V1 发布验收仍未完成。发布步骤见 `docs/release-checklist.md`。
