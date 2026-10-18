# 10 — 实现文件树、ignore 与链接目录

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` 第2/3节、`docs/architecture.md` 第3节、`docs/adr/adr-0007-single-workspace-storage.md` 和GPUI Kit [Tree文档](https://gpui-kit.com/zh-CN/component/tree.md)。确认唯一Workspace和安全文件写入服务存在。

只实现发现、文件树、普通路径操作与链接解析；不建 SQLite 搜索索引，不实现反向链接或自动改链接。

## 产物

- `omw-local/src/{ignore,scan,identity}.rs`（复用已有 identity）
- `omw-app/src/ui/file_tree.rs`
- `omw-local/tests/files.rs` 的发现/链接夹具
- 固定的内置 ignore fixture

## 固定规则

- 用 `ignore::gitignore::GitignoreBuilder(root)`，只加入根 `.ohmyworkignore` 与明确内置规则；禁止读取 `.gitignore`、global gitignore 或 `.git/info/exclude`。
- 内置至少排除 `.git/`、`.ohmywork/` 和应用临时文件；不默认排除用户 `.pi/`。用户规则文件不存在时不自动写。
- 按系统语义跟随 symlink/Junction，允许根外目标；规则匹配 logical path。

## 实现顺序

1. 自有遍历器生成 `logical_path → resolved target` alias；目录递归栈用 Windows volume+FileId 防环，全局目标缓存只减少重复工作。打开文件用解析后的实际路径+alias表归一成 `DocumentKey`，不要把会因原子替换改变的FileId当永恒文档身份。
2. 正确实现 gitignore 风格否定/父目录规则；“显示排除项”只影响树查看，不让它们进入未来搜索。
3. UI使用GPUI Kit `Tree`/`TreeState`及其展开、选中、键盘导航；目录展开时异步加载子节点。领域扫描/alias模型独立于组件状态，稳定TreeItem ID来自logical path，不把整棵物理树复制进UI。
4. 提供新建目录/Markdown、移动、重命名、在系统中显示；错误不留下 UI 假状态。
5. 普通链接：相对路径从当前文件父目录；`/Atlas/a.md` 从唯一 root；HTTP/HTTPS 交默认浏览器；其他类型交系统。
6. 不生成 heading slug，不建引用图，移动/重命名时不改其他 Markdown。

## 测试

`.gitignore`无效、`.ohmyworkignore`有效；否定规则；中文/长路径；Junction自环与多路径同目标；根外链接；Archive可浏览；文件操作失败回滚；链接解析。用Kit `test-support`覆盖Tree展开/折叠、方向键和选中；断言不自动改正文链接。

## 完成定义

扫描终止、有界、别名关系明确，文件树可操作。循环检测不能只靠路径字符串；无法读取单一路径只生成局部诊断，不清空整个树。