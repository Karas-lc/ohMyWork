# 26 — 建立可复现性能验证并针对性优化

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` 第9节、`docs/architecture.md` 第8节。确认所有主要V1运行路径已实现且诊断不泄露内容。

本步先测量再优化；不得为过基准删除短中文搜索、外部一致性、可访问性或把内存移到worker后隐瞒。

## 产物

- `tools/generate-corpus.py`：固定seed生成约1万篇/200MB/2万Task，混合中英文与三分区
- `crates/omw-app/src/bin/perf_runner.rs`（不进安装包）
- `tools/bench.ps1` 与报告模板/结果文件
- 仅针对已profile热点的代码改动和回归测试

## 测量环境记录

记录commit/版本、Rust、`gpui-kit`及其`gpui-pre`锁定版本和启用features、release locked构建、Windows build、CPU/RAM、实际渲染GPU/驱动、数据盘、供电、DPI、显示模式、杀毒/后台状态。附件、Pi、preview worker和GPU显存单列。

## 固定方法

1. 启动：热启动30次；冷启动只在明确清缓存/重启条件下单列。终点是可交互，不是窗口出现。
2. 搜索：索引就绪后固定100条长/短查询，多次运行；首批可显示时间包含20ms debounce。
3. 输入：固定普通页面，中文/英文操作至少1000次，测输入到显示；只能测layout时明确标为代理指标，不冒充呈现。
4. 稳态内存：10标签、默认画布、索引就绪、空闲30秒后测主进程PrivateBytes；另测预览压力、worker、Pi和总量。
5. 首次索引单独报告吞吐/峰值/可交互性。

## 目标

release：启动P95≤2s；搜索首批P95≤100ms；输入显示P95≤50ms；主进程稳态PrivateBytes≤500MiB。

## 执行顺序

生成可校验语料→校验结果正确性→采基线→profile CPU/分配/I/O/GPU→区分应用、自定义编辑器与Kit组件开销→列出最大真实热点→逐个做最小优化→每次跑功能回归和重测。先检查是否误启用tree-sitter/shell/webview/AllAssets等无关feature或资源，再改代码；缓存必须有界，后台任务保持代次/取消。

## 完成定义

报告包含原始样本、P50/P95、失败/离群规则、跳过项和主/子进程资源。未达标就如实列阻塞与profile证据；不能改测量起点或数据集声称通过。