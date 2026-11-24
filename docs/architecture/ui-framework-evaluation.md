# UI 框架选型：候选收敛与原型验收

> Decision preparation / Draft 1  
> 日期：2026-11-24  
> 当前状态：未选定框架；进入同规格原型阶段

## 1. 本轮判断

候选从“看起来能做桌面应用”收敛为两个 Rust 原生主候选 + 一个成熟终端 UI 对照组：

1. **Iced 0.14：目前最值得先做纵向原型。** 原因不是它最漂亮，而是它已经提供 PaneGrid（分割、拖动调整、拖放重排、程序化 split/swap/resize）、输入法支持、headless testing 和 end-to-end testing。FinalShell 最核心的分屏交互与这些能力高度重合。
2. **GPUI / GPUI Community Edition：保留为高交互密度候选。** GPUI CE 是面向独立应用的社区维护 fork，文档列出 macOS Metal、Linux X11/Wayland、Windows Win32/DirectX 后端；但 pre-1.0、生态和 API 稳定性仍是主要成本。必须确认所选版本与官方 Zed GPUI 的差异，不能混用两套 API 文档。
3. **Tauri 2 + xterm.js：作为终端体验基准和风险对照。** xterm.js 有成熟的 Unicode、IME、GPU renderer、selection、addon 生态；但前端 UI、WebView 与 Rust 后端之间形成额外边界。若 Rust 原生方案的终端绘制/IME 质量达不到预期，它是现实备选，而不是失败路线。

egui 暂时降为低成本对照，不优先投入。它适合快速验证状态模型和自绘能力，但在连接树、文件列表、复杂编辑器、多窗口和原生桌面交互上的总体工程成本尚未证明更低。

## 2. 证据与限制

### Iced 0.14

官方 0.14 release notes 已列出：
- Input method support。
- Headless mode testing 和 first-class end-to-end testing。
- PaneGrid drag/drop、拖到边缘分割。
- Text editor、smart scrollbars、offscreen rendering/screenshots。
- PaneGrid 支持横/纵 split、active pane、鼠标 resize、drag/drop、hotkeys、程序化 split/swap/resize。

这些是直接匹配产品需求的能力，但不等于本项目所有 UX 已开箱即用。仍需实测：
- PaneGrid 能否表达原版需要的分割树和 pane identity 稳定性。
- 大量连接树节点和 10,000 行文件列表的虚拟化性能。
- 中文 IME preedit/candidate window 定位，尤其 Linux Wayland 和 Windows。
- 自定义终端 renderer 的绘制 API、glyph atlas、DPI 和字体 fallback。
- 托盘、原生菜单、文件拖放和多窗口生命周期需要的第三方 crate/平台代码。

### GPUI / GPUI CE

GPUI 的强项是 GPU 自绘、复杂工作区/编辑器交互、实体状态和直接 Rust 事件处理。GPUI CE 文档列出 macOS Metal、Linux X11/Wayland、Windows Win32/DirectX 后端，定位是 standalone application fork。

风险：
- upstream GPUI 和 GPUI CE 不是可随意互换的 crate；API、feature、版本和依赖图必须固定到单一来源。
- pre-1.0 与快速演进意味着升级成本。
- tray、原生菜单、辅助功能、IME 和文件拖放仍须用真实桌面会话验证。
- GPUI 视图/元素的自绘终端 surface 要做性能测试，不要假设 Zed 编辑器的性能自动等于终端滚动性能。

### Tauri 2 + xterm.js

xterm.js 官方说明其支持 CJK/Emoji/IME、GPU renderer、selection、screen reader、addons，且被 VS Code、Tabby 等产品使用。它不是 shell/PTY，仍需 Rust 侧 PTY/SSH 数据桥。

风险：
- WebView 和 Rust 间的输出吞吐、背压、resize 同步、焦点/快捷键传递。
- macOS WKWebView、Windows WebView2、Linux WebKitGTK 的行为差异。
- 前端工程链、CSP、插件权限和 IPC 生命周期管理。
- 如果保留 Ghostty VT，就可能出现双终端解析器；如果使用 xterm.js，则需决定 Rust 是否还需要 libghostty-vt。两套 VT parser 不应无理由同时存在。

## 3. 原型必须使用同一产品模型

原型不应只做 hello-world。每个候选至少实现：

- MainWindow：左连接树、中间 terminal workspace、右侧 monitor tabs、底部 quick command bar。
- Workspace：2x2 pane layout；pane 可 split、resize、swap、drag-drop、close、focus。
- Pane identity：UI pane ID 与 shell session ID 分离；pane 移动后 session 不变。
- Connection tree：1000 条虚拟数据，搜索、折叠、拖放排序。
- File list：10,000 行虚拟数据，排序、筛选、滚动。
- Secondary windows：连接编辑器、设置窗口；同类型重复打开时聚焦已有窗口。
- IME：中文拼音 preedit、候选窗定位、焦点切换、resize 后定位。
- Input routing：Ctrl/Cmd shortcuts 与 terminal raw input 不冲突。
- DnD：连接树内部移动；本地文件拖进终端后作为安全 quoted path 粘贴。
- Theme/DPI：深浅主题、字体 fallback、100%/150%/200% DPI。
- Accessibility：键盘可达、可读标签和 screen reader 基础验证。

不在此阶段接 SSH 或远端文件操作。终端 pane 先用确定性 fake stream 和 VT fixture，隔离 UI 性能与网络波动。

## 4. 统一验收与评分

每项按 0-5 分评分，并附复现步骤/平台/视频或截图。没有实际验证的项目标为 N/V，不得按 0 或 5 估算。

| 维度 | 权重 | 验收标准 |
|---|---:|---|
| 分屏与 pane identity | 20% | 分割、拖动、交换、关闭、重排后 session identity 稳定 |
| 终端渲染与输入 | 20% | 60/120Hz 滚动、CJK/emoji、selection、鼠标、键盘协议 |
| IME 与文本输入 | 15% | preedit/candidate 定位正确；焦点切换不丢字、不重复 |
| 大列表性能 | 10% | 1000 节点树、10k 文件列表虚拟化 |
| 多窗口/快捷键 | 10% | 窗口生命周期、平台快捷键、焦点范围正确 |
| 跨平台/打包 | 10% | 五目标构建、依赖安装、安装包体积、启动与升级 |
| 拖放/剪贴板/托盘 | 10% | 文件 DnD、剪贴板、托盘菜单与恢复 |
| 可访问性/主题 | 5% | 键盘可达、语义树、DPI、字体与主题 |

决策规则：
- 五平台任何一个目标无法构建：该候选不能成为当前五平台主框架，除非明确有可维护的条件编译替代方案。
- 分屏 identity、IME 或终端输入有严重缺陷：不以性能或视觉优势抵消。
- UI 线程不能被网络/磁盘/VT 处理阻塞。
- 必须能在 headless CI 执行状态与消息测试；真实桌面 IME/托盘测试另设平台 smoke。

## 5. 实施顺序

1. **Iced 0.14 原型先行**：复用独立 fake terminal model，实现 PaneGrid、连接树、虚拟列表、二级窗口、快捷键和消息流。
2. **GPUI CE 对照原型**：实现相同布局与分屏；固定明确 crate source/version，禁止直接追 latest。
3. **Tauri + xterm.js 只做终端 surface 对照**：用相同输出 fixture 测滚动、IME、Unicode、输入延迟和 resize，不复制整个产品 UI。
4. 在 macOS、Windows、Linux X11、Linux Wayland 真实桌面分别跑 UI smoke；Linux ARM64 先保证构建与基本运行。
5. 评分和记录后写 ADR 决策；未通过决策门前不把 UI 框架依赖合并进主 workspace。

## 6. 当前不做的事

- 不先创建十几个业务 crate。
- 不把 terminal emulator 和 PTY/SSH session 混成一个类型。
- 不把 pane ID 当作 session ID。
- 不用 headless CI 通过推断 IME/托盘/拖放已通过。
- 不以单一平台截图或 benchmark 决定跨平台框架。
- 不在 UI spike 里实现真实 SSH、配置持久化或云同步。

## 7. 参考资料

- Iced 0.14 release notes：https://github.com/iced-rs/iced/releases/tag/0.14.0
- Iced PaneGrid API：https://docs.rs/iced/latest/iced/widget/pane_grid/struct.PaneGrid.html
- GPUI Community Edition：https://docs.rs/crate/gpui-ce/latest
- GPUI examples：https://gpui.rs/examples/
- xterm.js：https://github.com/xtermjs/xterm.js
- Tauri 2：https://v2.tauri.app/
