# 19 — 实现 Homepage 画布数学与交互

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` HOME-001..004、`docs/architecture.md` 第4节。确认App shell能承载Homepage Tab。

本步只做画布纯模型、坐标变换和通用交互；画布命中/变换需要自定义Element，但缩放、fit-all等可见命令使用GPUI Kit Button/Toolbar，不自制标准控件。具体组件/持久化在20。

## 数据模型与固定值

`CanvasState { version, pan:[f64;2], zoom:f32, widgets }`；widget含稳定UUID、kind、`WorldRect`、z、options。统一公式：`screen = viewport_origin + pan + world * zoom`。zoom初始限制 `0.25..=2.0`；默认组件最小尺寸从160×100逻辑单位起，可由kind覆盖。

## 产物

- `omw-app/src/home/{mod,model,transform,hit_test}.rs`
- 画布Element/交互控制器
- 数学属性测试和100组件夹具

## 实现顺序

1. 单一transform模块提供world↔screen，不在组件各自重写数学；拒绝NaN/Infinity/溢出尺寸。
2. 平移；以鼠标所在world点为锚缩放，调整pan使该点屏幕位置不动；支持键盘缩放/重置/fit-all。
3. z-order倒序hit-test；选中、抬升、拖动使用world delta；resize遵守最小尺寸。
4. 键盘移动默认10逻辑单位，精细1单位；所有操作有等价可访问命令。
5. 只渲染与视口相交及有限边缘缓冲的组件；初版线性过滤，100组件通过后再决定是否需要空间树。
6. 画布可负坐标、自由重叠和空状态；“无限”不意味着无界创建对象或非法坐标。

## 测试

- transform往返误差、负坐标、不同viewport origin、DPI、zoom边界。
- 鼠标锚点缩放不漂移；pan+zoom+drag组合。
- z-order命中、重叠选择、resize clamp、NaN/极值拒绝。
- 100组件下裁剪正确、键盘可完成选择/移动/fit-all。

## 完成定义

画布交互不依赖具体Task/Pi组件，所有坐标变化都通过纯模型可测。不要加入响应式网格、日历或插件组件系统。