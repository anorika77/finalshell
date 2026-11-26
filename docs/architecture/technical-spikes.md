# 技术验证：终端引擎与桌面 UI

> Technical Spike Plan / Draft 1  
> 调研日期：2026-11-24  
> Rust 仓库：anorika77/finalshell  
> 上游行为基线：TooonyChen/finallyshell @ 78c2f91603bfb014b800a88cfb68fded64934141

## 1. 本轮结论摘要

**终端引擎：把 libghostty-vt 1.0.1 作为首选验证对象，但暂不承诺五平台运行支持。** 官方 C ABI 从 1.0 起稳定；1.0.1 Rust crate 由第三方维护，是基于旧 Rust binding 的 fork，且 docs.rs 明确写明当前主要由 gpui-cn 使用、macOS 是该版本唯一声明支持的 runtime。其余目标即使能够构建，也不代表终端行为已受支持。

**UI：先把 GPUI、Iced、Tauri 2 放入短名单；egui 作为轻量对照组。** 暂不定框架。GPUI 的复杂编辑器类交互与自定义绘制值得优先验证；Iced 有清晰的消息驱动模型和多窗口能力；Tauri 2 + xterm.js 是成熟终端 UI 的现实基准，但引入 Web 前端；egui 容易做出可运行原型，但要验证大型信息密集型桌面应用的交互和视觉上限。

**关键架构约束：libghostty-vt 只负责 VT 状态机和渲染快照，不负责 PTY、SSH、窗口绘制或 shell。** 这几层必须分开。

## 2. 终端引擎评估

### 2.1 libghostty-vt 现状

官方文档当前列出的版本为 1.0.1：
- 1.0.0 于 2026-11-06 发布，C ABI 从 1.0 起稳定；1.x 次版本只增加 API，不移除/重排既有 ABI。
- 1.0.1 于 2026-11-11 发布，包含 Windows x86_64 和 ARM64 预构建包。
- 支持 Linux、macOS、Windows 等平台；官方说明 Linux 没有发布预编译包，需要源码构建。
- Rust sys crate 在 Linux 等无预构建库的目标上通过 Zig 从 pinned source/deps tarball 静态构建；要求 Zig 0.17.x。Linux x86_64/ARM64 必须验证构建耗时、内存和缓存。
- Windows 的预构建静态库是 GNU ABI；MSVC 默认应源码构建，或显式使用预构建库并通过 lld 链接（不能使用默认 link.exe）。这不是 DLL 部署问题，而是 ABI/COFF 链接器兼容问题。

核心 API/职责：
- `GhosttyTerminal`：接收 VT 字节流并维护屏幕、滚动历史、光标、样式和模式。
- `GhosttyRenderState`：生成可供 renderer 消费的快照，支持 dirty row 更新。
- 输入编码器：键盘、鼠标、焦点、粘贴。
- 搜索、选择、滚动历史、颜色、OSC/自定义序列和 effects。
- 不提供窗口控件，也不直接启动或管理 PTY。

### 2.2 Rust 接入路径

按优先级验证：

1. **候选 A：`ghostty-vt` + `ghostty-vt-sys` 0.9.0+ghostty-vt.1.0.1。** 它提供 Rust 封装，底层绑定 Ghostty 1.0.1；crate 文档说明该 fork 主要由 gpui-cn 使用，macOS 是当前唯一声明支持的 runtime。macOS 下载预构建库；Linux 等目标通过 Zig 源码构建；Windows MSVC 有特殊 lld/源码构建路径。将其视为 spike 依赖，不视为已验证的产品依赖。
2. **候选 B：直接绑定官方 C ABI。** 若 Rust wrapper 的线程/生命周期抽象不适合本项目，可用 bindgen 或手写最小 FFI 封装。ABI 稳定降低了绑定维护风险，但不免除指针、回调、allocator、对象销毁和线程安全审计。
3. 不把旧版 `libghostty-vt 0.2.2` 当作最终锁定依赖。当前 1.0.1 wrapper 仍明确标注所有 handle 为 `!Send` / `!Sync`，并要求在拥有线程驱动。该约束不是旧版本遗留，而是当前封装的实际线程契约。

### 2.3 线程模型

初始设计按每个 Terminal 一个 owner thread：
- SSH/PTY reader 把字节块投递到 terminal actor。
- Terminal actor 独占修改 `GhosttyTerminal`，处理 resize、输入模式、OSC/effects 和搜索请求。
- renderer/UI 不直接访问正在变化的 Terminal；由 owner thread 生成 RenderState/不可变帧数据，再投递给 UI。
- 键盘/鼠标/粘贴输入经消息发回 owner thread，再由 Ghostty encoder 编码。
- terminal actor 退出时统一销毁 terminal、render state 和相关句柄。

这只是安全默认方案。最终要根据 1.0.x C API 的线程保证和 Rust wrapper 的 trait 实现验证；不得通过 unsafe impl Send/Sync 强行绕过。

### 2.4 必须通过的终端 spike

一个独立 example，不依赖 GUI：
1. 创建 terminal，写入纯文本、CSI/SGR、OSC title、alternate screen、鼠标模式和 UTF-8 宽字符。
2. 创建 RenderState，验证 dirty rows、cursor、颜色、属性、scrollback。
3. resize（同时改变 rows/cols），验证 reflow 和滚动历史。
4. key/mouse/paste encoder 往返，覆盖 Ctrl/Alt/Shift、功能键、Kitty 键盘协议和 bracketed paste。
5. 搜索、选择、复制和滚动。
6. 多个 terminal 实例并行运行；每个实例在自己的 owner thread；销毁/重建循环和压力测试。
7. 测试 Terminal 对象、RenderState 和帧数据的 Send/Sync 边界；禁止以编译通过代替并发正确性。
8. 五个目标平台均执行 build + test；Windows 必须明确 MSVC/GNU ABI、DLL 部署和运行时加载方案。

**通过标准：** 五平台构建成功；Linux/macOS/Windows 原生 runner 的终端核心行为测试通过；没有不受控下载或运行时找不到 DLL；线程归属和销毁规则有可测试约束。

## 3. UI 框架候选对比

| 候选 | 优势 | 已知风险/代价 | 在本项目中的定位 |
|---|---|---|---|
| GPUI | Rust 原生；为复杂编辑器/工作区设计；自绘与高性能交互；文档列出 macOS Metal、Linux X11/Wayland、Windows Win32/DirectWrite 后端 | pre-1.0、快速演进；API/内部接口变动；系统托盘/原生菜单/辅助功能要单独验证；Linux X11/Wayland 双后端依赖 | 首选技术 spike |
| Iced 0.14 | Rust 原生；消息驱动架构；多窗口；wgpu/tiny-skia renderer；0.14 发布说明列出输入法支持、table/grid、PaneGrid 和端到端测试 | 复杂拖拽分屏、树/表格、富文本编辑器仍需原型验证；原生托盘需额外组件；必须验证实际五平台后端组合 | 第二候选，作为消息驱动架构对照 |
| Tauri 2 + xterm.js | Web UI 组件生态成熟；xterm.js 的终端输入/渲染/选择/搜索生态强；Rust 后端可承载 SSH/配置；跨平台打包成熟 | UI 不再是纯 Rust；系统 WebView 差异；WebView 与 Rust 的高频终端帧/输入桥接需测；前端依赖链和安全边界更复杂 | 终端体验对照组/保底路线 |
| egui 0.37 | Rust 原生、原型速度快；多视口、IME/剪贴板/AccessKit 进展；便于自定义绘制 | immediate-mode 与复杂桌面信息架构的状态组织成本；树/表格/编辑器细节可能要大量自建；视觉和原生感需实测 | 低成本对照，不作为默认首选 |

### 3.1 评估原则

不能仅凭 demo 能打开窗口就选型。FinalShell 的 UI 至少包含：
- 多窗口、主窗口/连接管理器/选项/文件编辑器等窗口生命周期。
- 左侧连接树、中心多标签终端、可左右切换的监控/文件/进程/网络面板、底部命令编辑器。
- 可拖动的二叉分屏树、pane 交换、pane 与 shell 的稳定绑定。
- 大量键盘快捷键、终端焦点与普通控件焦点切换。
- 中文输入法、候选窗定位、组合输入、复制粘贴和拖放。
- 文件列表、编辑器、表格、菜单、弹窗、托盘和系统主题。
- 5 个发布目标：macOS ARM64/x86_64、Linux x86_64/ARM64、Windows x86_64。

## 4. UI 最小统一原型

每个候选实现同一张测试界面，不做产品 UI：
1. 主窗口三栏布局：连接树 / 中央终端 / 右侧监控。
2. 中央区域 2×2 分屏，支持拖动 divider、拖动 pane、交换 pane、关闭 pane。
3. 左侧 1000 条连接的树形列表，支持搜索、折叠、拖放排序。
4. 右侧可切换终端/文件/进程/网络 tab，文件列表 10,000 行虚拟化滚动。
5. 额外窗口：连接编辑器与选项窗口；重复打开同类窗口要聚焦已有窗口。
6. 快捷键：Cmd/Ctrl+W、Cmd/Ctrl+T、pane focus、pane split、搜索、复制粘贴。
7. 中文 IME：拼音组合、候选窗位置、焦点切换、窗口缩放后候选位置。
8. 拖放：文件拖进终端触发路径粘贴；连接树内部拖放移动。
9. 托盘：显示/隐藏、退出、菜单动作；Linux X11/Wayland 分别记录。
10. 高 DPI、主题切换、字体 fallback、窗口最小尺寸和多显示器。

统一记录：构建耗时、冷启动、空闲内存、1000/10000 行列表滚动、终端帧延迟、输入响应、平台缺陷和需要的 workaround。

## 5. 预期依赖方向与架构约束

- `terminal-core`：只包装 libghostty-vt，提供帧快照/输入编码/搜索/选择等接口。
- `terminal-session`：SSH shell channel、本地 PTY/ConPTY 与 terminal-core 的数据桥。
- `terminal-view`：UI 框架专属渲染适配；不拥有 SSH channel。
- `application`：管理 pane/session 领域状态和生命周期；不直接操作 VT C handle。
- UI 框架事件通过 application commands 发送；后台结果通过有界 channel/事件队列返回。
- 一个 pane 绑定一个 terminal instance 和 shell channel；同一 SSH client 可以服务多个 pane/channel。
- terminal 输出必须有背压/批量合并策略；UI 不得每个字节触发一次重绘。

## 6. 当前技术判断与决策门

### 可以确定
- libghostty-vt 1.0.x 是比旧 0.2.2 更合理的终端核心候选：官方 C ABI 已稳定，终端状态、render snapshot、输入编码、搜索/选择/滚动历史均有对应 API。
- Terminal 与 renderer/PTY/SSH 是分离职责；应以 owner-thread actor + render snapshot 做第一版边界。
- 五平台不应靠本地交叉编译验证；继续使用 GitHub Actions 原生 runner。
- UI 框架必须经过真实分屏/IME/拖放/多窗口测试。

### 尚不能确定
- `ghostty-vt` Rust wrapper 是否适合作为长期依赖，尤其是 Windows MSVC、Linux ARM64、Zig 构建和跨线程 trait 边界。
- GPUI 是否适合本项目的完整窗口/托盘/辅助功能和 Linux 桌面支持。
- Iced 的复杂分屏、虚拟化列表、树、拖放和自定义终端绘制的开发成本。
- Tauri WebView 高频帧数据桥接是否能达到终端输入/滚动体验要求。

### 决策门
只有当终端 spike 和 UI prototype 都达到上面的验收标准，才冻结依赖和 crate 结构。若候选不通过，保留第二路线，不在主分支留下半成品 UI 依赖。

## 7. 官方资料与调研依据

- libghostty-vt 主页：https://lib.ghostty.org/vt/index.html
- ghostty-vt 0.9.0+ghostty-vt.1.0.1 Rust wrapper：https://docs.rs/ghostty-vt/latest/ghostty_vt/
- ghostty-vt-sys 构建矩阵与 MSVC 说明：https://docs.rs/crate/ghostty-vt-sys/latest
- ABI 稳定性：https://lib.ghostty.org/vt/stability.html
- 1.0.1 发布说明：https://lib.ghostty.org/vt/release-notes.html
- 安装与平台构建：https://lib.ghostty.org/vt/installing.html
- Terminal API：https://lib.ghostty.org/vt/group__terminal.html
- RenderState 与快照渲染：https://lib.ghostty.org/vt/guide-rendering.html
- Rust wrapper：https://docs.rs/ghostty-vt/latest/ghostty_vt/
- Rust sys crate：https://docs.rs/ghostty-vt-sys/latest/ghostty_vt_sys/
- GPUI README：https://github.com/zed-industries/zed/tree/main/crates/gpui
- Iced 0.14 release notes（IME、table/grid、PaneGrid 等）：https://github.com/iced-rs/iced/releases/tag/0.14.0
- Iced：https://github.com/iced-rs/iced
- Iced FAQ / IME：https://book.iced.rs/faq
- egui changelog：https://github.com/emilk/egui/blob/main/crates/egui-winit/CHANGELOG.md
- Tauri 2：https://v2.tauri.app/

## 8. 下一步执行

1. 在独立 spike 目录建立 `terminal-engine-smoke`，锁定 `ghostty-vt` wrapper 版本并验证 Linux x86_64。
2. 给 GitHub Actions 增加临时 spike matrix，覆盖 macOS ARM64/x86_64、Linux x86_64/ARM64、Windows x86_64；先不把依赖合并进主应用。
3. Windows runner 单独验证 MSVC DLL 加载；Linux ARM64 验证 Zig 0.17 构建和缓存。
4. 同步建立 GPUI/Iced/Tauri 的 UI prototype 分支或独立目录，复用同一份验收清单。
5. 记录实测数据后形成 ADR，选定 UI 与终端依赖，再进入 domain/config-store 业务切片。
