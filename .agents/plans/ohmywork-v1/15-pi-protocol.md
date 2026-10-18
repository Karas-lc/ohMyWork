# 15 — 实现 Pi 协议纯逻辑与假进程

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/pi-integration.md`，并在实现前读取用户目标版本的官方 Pi RPC/Extension UI 文档；记录版本来源。不得启动真实模型任务来“探测”协议。

本步只实现与OS启动无关的 JSONL codec、wire类型、reducer和可控假进程；真实Pi发现/启动在16，业务UI在17。

## 产物

- `crates/omw-pi/src/{lib,codec,wire,reducer}.rs`
- `crates/omw-pi/tests/{framing,lifecycle,ui_requests}.rs`
- `crates/omw-app/src/bin/fake_pi.rs`（测试/基准target，不进安装包）
- 最小RPC fixture

## 协议契约

1. `ByteFramer` 累积原始bytes到 LF，去可选 CR 后才 UTF-8/JSON decode；U+2028/U+2029不分帧。设置单帧/缓冲上限并返回类型化错误。
2. serde定义已知 command/response/event/request；未知普通event保留类型诊断后忽略，未知交互request必须能回复取消/错误，不能悬挂。
3. request_id→oneshot只关联response；event独立分发，不能假定顺序。
4. 消息按 contentIndex 累积 text/thinking/tool block；`message_end` 用完整最终值替换增量结果。
5. prompt response `disposition=handled` 不等待不存在的run；`agent_end` 不代表idle，`agent_settled`才表示当前自动工作结束。
6. 状态纯 reducer：Stopped→Starting→Idle/Running→Closing→Stopped/Failed。非法/迟到事件有确定处理。
7. 不在模型中加入宿主queue/steer/审批策略。

## fake_pi

通过脚本参数选择：拆帧、乱序response/event、流式消息、handled无run、settled、Extension UI、stderr洪泛、异常退出、永不退出。使用与真实适配器相同stdin/stdout边界。

## 测试

拆半多字节UTF-8、U+2028、CRLF、多帧同读、超限/坏JSON；out-of-order response；最终消息覆盖；stdout/stderr背压；未知event/request；handled与settled状态；断开时所有pending请求结束。

## 完成定义

纯逻辑测试无需真实Pi、网络或模型即可稳定复现生命周期。日志/断言不包含完整用户prompt。后续只能通过公开 client/reducer事件接UI，不解析stdout字符串。