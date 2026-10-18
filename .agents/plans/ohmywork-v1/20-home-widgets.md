# 20 — 实现 Homepage 组件与布局保存

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` HOME-001..004、`docs/architecture.md` 第4节和GPUI Kit组件目录。确认19的CanvasState/交互和既有文件、Task、Pi服务API可用。

本步实现内置组件和画布局部持久化；完整App状态恢复/便携切换在23。

## 组件范围

仅：Atlas/Efforts/Archive入口、最近页面、置顶页面、基础Task、Pi状态/入口、快捷动作。无日历、多Workspace、插件或用户脚本组件。所有起始组件可删除，空画布能重新添加。

## 产物

- `omw-app/src/home/{widgets,persist}.rs`
- versioned Canvas JSON schema、导入/导出和夹具
- `%LOCALAPPDATA%/ohMyWork/ui-state.json` 中独立的canvas段（与23可扩展schema兼容）

## 实现顺序

1. 为每种 `BuiltinWidgetKind`定义最小尺寸、可访问标题、options schema和服务读取接口；组件内部优先组合Kit Button/Checkbox/Badge/VirtualList等标准控件，画布只负责世界坐标定位、z-order和裁剪。组件只引用领域服务，不复制事实。
2. 提供可删除起始布局和“添加组件”命令；最近/置顶只持路径引用，Task来自索引，Pi只显示状态/入口。
3. 内部状态变更通过节流的安全临时文件+替换持久化；不能存Markdown正文、Pi transcript或未保存编辑。
4. 导出versioned JSON；导入先解析到临时结构，校验version、kind、数量上限、UUID、有限坐标/尺寸、options，再整体替换。
5. 导入成功提供一次布局Undo；失败保持原布局。损坏磁盘布局进入可操作安全默认，不覆盖坏文件以掩盖问题。
6. 缺失路径显示可关闭/可修复placeholder，不阻止其他组件。

## 测试

空画布添加；起始布局全删；组件服务更新；Kit控件在平移/缩放容器中的焦点与键盘操作；重启恢复canvas；导入未知kind/version、重复ID、NaN、超量、畸形options；失败不丢原布局；导出无正文/对话/本机凭据；断言没有日历/多Workspace。

## 完成定义

Homepage是可编辑自由画布，内置组件范围准确，JSON只是导入/导出/迁移边界，不成为实时监听配置。