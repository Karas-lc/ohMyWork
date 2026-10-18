# 16 — 实现 Windows Pi 启动与会话

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/pi-integration.md` 和目标Pi版本官方 CLI/RPC/session文档。确认15的 fake_pi、codec和生命周期测试通过。真实兼容测试不得修改用户Pi配置或发送模型prompt。

本步实现程序发现、进程I/O、cwd、新建/恢复和关闭；附件/Extension UI在17。

## 产物

- `omw-pi/src/{launch,client,session,shutdown}.rs`
- `omw-app/src/ui/pi_panel.rs` 的连接/状态骨架，使用GPUI Kit Sidebar/Input/Button/Dialog组合
- `tests/lifecycle.rs` 的 Windows/fake进程测试

`PiClient`运行在应用后台Tokio runtime（固定2个worker线程）；不得在GPUI Kit/GPUI主线程调用 `Runtime::block_on`，所有回调返回UI前检查请求/会话代次。

## 启动模型

`PiLaunch { executable: PathBuf, args: Vec<OsString>, cwd: PathBuf, session: NewOrResume }`。参数始终是数组，不用空格split，不把prompt拼入shell。

1. 从PATH解析用户Pi或接受显式程序；`.cmd` 使用专门的安全shim launcher，严格处理空格/中文/引号。
2. 用户明确选择 cwd；可在Workspace外。检查目录可用，但不施加ohMyWork ignore/权限。
3. 用户选择New或通过系统文件选择器选择已有session；不扫描认证/私有配置猜历史。可记最近明确选择路径，但不自动启动/恢复。
4. 只添加官方RPC模式和用户选择的session参数；不注入system prompt、工具白名单、no-extensions、approve或队列选项。
5. 用户点击启动后才spawn；异步持续消费stdout/stderr，UI线程不block。用官方只读state命令核验连接，不发探测prompt。
6. 关闭：运行中发abort→最多2秒等响应→关stdin→最多3秒等退出。超时显示“未退出”，只有用户显式操作才终止该子进程/其受控进程树；不杀所有同名进程。
7. stderr默认不原文落日志；错误区分找不到、目录、权限、协议、配置/认证、退出码。

## 测试

fake_pi覆盖启动、abort、有序退出、超时强停、异常退出、stderr背压；路径含空格/中文；npm `.cmd` shim；无程序/无权限/坏cwd；New/Resume参数；不同cwd。真实Pi只做明确版本的无副作用连接/state验收。

## 完成定义

用户可选择任意cwd和New/Resume并看到可靠状态；关闭不会留下宿主管理的stdin或pending request。不得安装、升级、修改或限制Pi。