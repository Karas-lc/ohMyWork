# 12 — 实现中文短词搜索

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` SEARCH-001/002与性能预算、`docs/architecture.md` 第3/8节，以及GPUI Kit Input/VirtualList文档。确认11的单线程索引、两张FTS和alias/area schema已运行。

本步只实现搜索正规化、查询和UI；不改文档事实、不索引图片/PDF、不做语义搜索。

## 产物

- `omw-local/src/index/{normalize,grams,query}.rs`
- `omw-app/src/ui/search.rs`
- 中文/英文/特殊字符搜索夹具与首次基准记录

## 固定算法

- 文件名、路径、标题、Markdown文本投影统一做 NFKC + Unicode lowercase；保留从正规化结果回到原文片段所需映射，或命中后在原文二次定位。
- 长度按 Unicode scalar 计：≥3 走 `search_long` trigram；1字符 token 为 `u<hex>`；2字符 token 为 `b<hex>_<hex>`。
- ingest 为每文档生成去重 unigram/bigram ASCII token 集合写 `search_short.grams`。
- FTS MATCH 输入按字面短语安全引用，SQL参数化，不允许用户注入操作符。
- 候选必须用正规化字段 `contains` 复核，再按 alias 过滤 ignore/Archive。

## 查询与UI

1. 用长期存在的Kit `InputState`/`Input`承载查询，用`VirtualList`渲染结果；首批50条，文件名/标题命中优先，正文其次，最后按路径稳定排序；支持分页，不为精确总数阻塞首批。
2. 同一 resolved目标多alias只显示一次，优先非Archive可见 alias；用户可显式包含Archive。
3. 片段按需生成并正确映射原文，不能把正规化位置直接当 byte offset。
4. 每次查询有 generation/cancel token；默认20ms debounce。旧结果不得覆盖新输入。
5. UI显示路径、标题/片段、Archive状态和索引尚未就绪状态。

## 测试

精确预期集合覆盖：“中”“中文”“项目方案”、ASCII大小写、全角、组合Unicode、Emoji、引号、百分号、FTS语法字符、重复alias、ignore、Archive开关、分页和过期结果。用Kit `test-support`驱动输入/键盘选择/滚动；验证短中文没有回退为全库正文扫描。

用固定生成语料记录首批结果 P95 的当前值与测量方法；此时只记录，不提前声称达到最终预算。

## 完成定义

1/2字符中文与≥3字符查询都返回正确结果，用户字符串不能改变SQL/FTS语义，UI取消可靠。若片段映射不可信，显示无片段结果也好过错位/越界。