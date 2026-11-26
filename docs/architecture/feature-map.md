# 原版功能地图与 Rust 模块映射

> 状态：Architecture Discovery / Draft 1  
> 规格基线：TooonyChen/finallyshell，commit 78c2f91603bfb014b800a88cfb68fded64934141  
> 本文记录功能、边界和兼容性要求，不代表 Rust 实现已经存在。原 Go 仓库只读；本仓库独立实现，不复制原项目源文件。

## 1. 规模与拆解原则

规格基线包含 278 个 Go 文件、其中 119 个测试文件，约 88,639 行 Go 代码。主要业务集中在 internal/app，其余包承担配置、SSH、终端编码、SFTP、监控、同步、导入和资源处理。

迁移原则：

1. 按用户可观察行为迁移，不按 Go 文件逐个翻译。
2. 先建立稳定领域模型和协议边界，再接 UI；避免 UI 状态直接拥有 SSH、SFTP 或文件存储细节。
3. 配置、凭据、同步加密格式是数据兼容边界；终端、SSH 会话和分屏是运行时边界。
4. 后台任务必须有取消、超时、关闭和结果回投规则；UI 线程不得执行阻塞网络、磁盘或凭据操作。
5. 五个平台是持续验收矩阵：macOS ARM64/x86_64、Linux x86_64/ARM64、Windows x86_64。Linux ARM64 与 x86_64 分别原生构建。
6. 第一阶段保持模块化单体，不为“看起来整齐”提前拆出大量空 crate。

## 2. 产品功能地图

| 功能域 | 用户可见能力 | 原版主要实现位置 | Rust 目标边界 |
|---|---|---|---|
| 应用与窗口 | 主窗口、子窗口、单实例、窗口聚焦/恢复、关闭确认、托盘、退出 | main.go、tray.go、rendering.go、internal/app/windows.go、closing.go | desktop-shell + platform-adapter |
| 会话与连接管理 | 新标签、快速连接、最近连接、连接树、文件夹、搜索、排序、重命名、复制、删除/恢复、批量连接 | internal/app/session.go、start.go、conntree.go、manager.go、config/hosts.go | session-manager + connection-domain + config-store |
| SSH 认证与信任 | 密码/私钥/SSH Agent、known_hosts、首次指纹确认、指纹变化拒绝、保活、错误分层 | internal/sshx/*、internal/app/dialogs.go、secrets.go | ssh-transport + credential-store |
| 代理与跳板机 | 直连、SOCKS5、HTTP CONNECT、多级跳板、循环检测、逐跳认证和错误归因 | internal/config/proxy.go、internal/sshx/proxy.go、internal/app/proxy.go | ssh-transport |
| 终端 | 远程交互终端、本地终端、TERM/按键/编码、查找、全屏、字体/配色/背景、鼠标和快捷键 | internal/app/terminal.go、local.go、findbar.go、appearance.go、mouse.go、keyseq.go、internal/termenc | terminal-core + terminal-session |
| 分屏与面板布局 | 二叉分屏树、横/竖分屏、拖动/交换/关闭、焦点分屏、独立面板标签、左右/底部面板布局 | internal/app/panes.go、paneview.go、paneltab.go、splits.go | workspace-model（纯状态）+ desktop-ui |
| 命令输入与快捷命令 | 命令输入栏、补全/历史、服务器 shell 历史、快捷命令分组、命令编辑器、发送到当前/多个会话 | commandbar*.go、quickcmds*.go、cmdeditor.go、sendtarget.go、config/history.go | command-tools + config-store |
| 文件管理与传输 | SFTP 浏览、目录树、软链接、文件操作、上传/下载、队列/进度/取消/重试、打包/解包 | internal/remotefs/*、internal/app/files.go、fileops.go、transfers.go、archive.go | remote-fs + transfer-manager |
| 远程文件编辑 | 文本/二进制识别、编码与换行符、保存、外部修改冲突检测、未保存修改保护、断线处理 | internal/remotefs/textfile.go、internal/app/fileeditor*.go | remote-file-editor（用例）+ remote-fs |
| 隧道 | 本地转发 -L、远程转发 -R、动态 SOCKS5 -D、启停状态、流量/连接数、失败原因 | internal/sshx/forward.go、internal/app/tunnels.go、config/tunnels.go | ssh-transport + tunnel-manager |
| 服务器监控 | Linux uptime/load/CPU/内存/swap/磁盘/网卡流量/延迟曲线、系统信息 | internal/monitor/monitor.go、net.go、sysinfo.go、internal/app/monitorparts.go | server-monitor |
| 进程与网络 | 进程列表/详情/环境变量/信号、socket、端口和进程关联、路由追踪 | internal/monitor/procs.go、netstat.go、trace.go、internal/app/processes.go、network*.go | server-monitor + network-tools |
| IP 归属地 | ip2region 本地库、校验下载、在线查询、缓存 | internal/geo/*、internal/app/geo.go | geo-service |
| 云同步 | Gist/WebDAV/S3、自定义 SigV4、Argon2id + AES-256-GCM、增量三方合并、冲突重试、同步状态 | internal/cloudsync/*、internal/config/sync.go、internal/app/sync.go | cloud-sync + config-store + credential-store |
| 连接导入 | FinalShell、Xshell、SecureCRT、MobaXterm、OpenSSH config、CSV/XLSX；编码识别、预览、去重、文件夹导入 | internal/importer/*、internal/app/import*.go | connection-import |
| 文件图标主题 | 内置 Material/Seti、VSIX 安装、扫描 VS Code 系扩展、文件名/扩展名/语言匹配、字体转换 | internal/fileicons/*、internal/app/fileicons.go | file-icon-themes |
| 外观、语言与资源 | 浅/深色主题、图标、字体、背景图片、英文/中文文案、快捷键平台差异 | internal/icons、internal/widgets、locales、internal/app/palette.go 等 | ui-assets + localization |
| 更新与发布 | 更新检查/签名、版本信息、发布说明、第三方许可证清单、构建打包 | updates.go、tools/release、mygo.json | update-service + CI/release workflows |

### 2.1 功能层级与优先级

- **P0：运行骨架**：应用启动/退出、配置读写、凭据抽象、SSH 连接、host key 信任、终端输入输出、断线/重连、基本会话标签。
- **P1：日常核心工作流**：连接管理器、代理/跳板、SFTP 浏览和传输、分屏、命令栏、隧道。
- **P2：增强能力**：服务器监控、进程/网络页、远程文件编辑、导入器、快捷命令。
- **P3：生态与体验**：云同步、IP 归属地、文件图标主题、更新器、完整外观/快捷键配置、托盘与细节打磨。

这个优先级是迁移排期建议，不是删减功能的决定。最终兼容范围仍以功能清单逐项验收。

## 3. Rust workspace 模块映射

建议目标结构如下。先确定职责边界，不立即创建所有 crate。

| Rust crate（建议名） | 负责内容 | 不应负责 |
|---|---|---|
| app（binary） | 组装依赖、启动运行时、初始化日志/配置/平台适配、启动主窗口 | 业务规则、协议细节 |
| application | 用例编排：连接/断开/重连、导入、传输、同步、窗口级工作流；向 UI 发布事件 | 直接绘制 UI、直接操作平台 API |
| domain | Host/Folder/SessionId/PaneTree/Tunnel/Transfer 等领域类型、状态转换、不变量、领域错误 | JSON、SSH 库、GUI 框架类型 |
| config-store | hosts/settings/history/quickcommands 等持久化、迁移、原子写入、未知字段保留 | 凭据明文持久化 |
| credential-store | macOS Keychain、Windows Credential Manager、Linux Secret Service；内存降级；异步调用边界 | 普通设置和主机 JSON |
| ssh-transport | SSH 客户端、认证、known_hosts、保活、代理/跳板、多路复用通道、端口转发 | 终端渲染和 UI |
| terminal-core | VT 状态机/渲染模型、输入映射、搜索、终端配置；评估并封装 libghostty-vt | SSH 建连和窗口生命周期 |
| terminal-session | 把 SSH shell channel 或本地 PTY/ConPTY 接到 terminal-core；shell 生命周期、编码桥接 | GUI 布局 |
| remote-fs | SFTP 文件元数据、软链接、操作、上传下载、进度/取消、文本安全读写、归档传输 | 传输 UI |
| server-monitor | 远端采集脚本/解析、进程、网络 socket、shell cwd/前台程序、agent 检测 | 面板 UI |
| tunnel-manager | 隧道配置到运行实例的生命周期、状态和流量统计（底层转发依赖 ssh-transport） | SSH 握手实现 |
| cloud-sync | Envelope、KDF/AEAD、provider、三方合并、并发冲突重试 | 配置文件直接写盘和 UI |
| connection-import | 各来源解析、编码、预览数据、去重与导入计划 | 直接修改全局配置 |
| geo-service | xdb 读取/下载校验、在线查询、缓存 | UI 文案 |
| file-icon-themes | VSIX 安全读取、主题解析、图标匹配、字体资产转换 | 扩展代码执行 |
| platform-adapter | 文件/目录选择、系统信息、单实例、托盘、剪贴板、窗口/更新/凭据能力适配 | 业务决策 |
| desktop-ui | 主窗口、子窗口、会话/连接树、面板、分屏视图、编辑器和交互 | 网络/文件协议实现 |

### 3.1 依赖方向

目标依赖保持单向：

- app 负责组装 desktop-ui、application 和 platform-adapter。
- desktop-ui 依赖 application 的用例接口与 domain 的只读视图，不直接依赖 SSH/SFTP 实现。
- application 编排 domain、config-store、credential-store 接口、ssh-transport、terminal-session、remote-fs、server-monitor、tunnel-manager、cloud-sync、connection-import 和 geo-service。
- terminal-session 依赖 terminal-core 与 SSH/PTY 抽象；terminal-core 不依赖 SSH。
- tunnel-manager 依赖 SSH 转发接口；remote-fs 依赖 SSH/SFTP channel 抽象。
- cloud-sync 处理数据和远端 provider，不直接控制 UI 或配置文件写盘。

具体 crate 依赖要在技术验证后收敛；不为抽象而抽象，不提前增加泛化 trait crate。

## 4. 必须保持的兼容性契约

### 配置与数据
- hosts.json v1 是主机数组，v2 是含 version、folders、hosts 的对象；读取旧版必须迁移，不能丢主机。
- 对未知 JSON 字段必须保留并在保存时写回；新字段采用向后兼容默认值。
- 配置写入采用临时文件 + flush/sync + 原子替换；损坏文件先备份，再报告/恢复，不能静默覆盖。
- hosts、folders、settings、历史、quick commands、command editor 等文件的职责和本机/同步范围必须分清。
- 密码、私钥口令、代理密码、同步服务凭据不能写入普通 JSON；通过系统凭据库保存，能力不可用时只在内存中保留并明确降级。

### SSH 与会话
- 首次连接的 host key 必须显式确认并写入 known_hosts；已知 key 变化必须拒绝，不能自动接受。
- 密码、私钥、SSH Agent 都是认证路径；代理和每级跳板机各自有认证、host key 和错误上下文。
- 保活、断线检测、重连、会话关闭、子通道关闭必须有确定生命周期。
- 多个终端 pane 共享一个 SSH client，但每个 pane 拥有独立 shell/channel、终端状态和查找状态；会话级 SFTP/监控/隧道不应重复创建。
- 分屏树、分隔比例、会话重命名等哪些持久化、哪些只在本次运行有效，必须按原行为区分。

### 终端与文件
- 远端编码可配置（UTF-8、GBK/GB18030、Big5、Shift-JIS、EUC-JP/KR、Latin-1、Windows-1252）；终端内部统一处理 UTF-8。
- 本地终端必须使用平台 PTY：Unix PTY 与 Windows ConPTY；不能把远程 SSH channel 当作本地 PTY。
- SFTP 传输需要并发读写、进度、取消、覆盖确认、软链接规则和失败恢复。
- 远程文本编辑必须识别二进制文件、保留编码/换行符，并在保存时检测远端文件变化，避免无提示覆盖用户修改。

### 云同步
- 远端 Envelope 当前格式为 format 1：Argon2id 派生密钥 + AES-256-GCM；参数和 AAD 都属于格式契约。
- Payload schema 1 按 folder/host ID、setting key、secret key 分项三方合并；双边冲突遵循“后同步设备优先”的既有规则。
- provider 包括 Gist、WebDAV、S3 兼容存储；S3 签名、ETag/条件写入和冲突重试需要独立测试。
- 同步状态和 base 快照是合并正确性的必要组成，不能只实现“上传/下载整个 JSON”。

## 5. 最高风险技术验证（先于大规模实现）

| 风险 | 为什么是架构级问题 | 必须验证的最小实验 |
|---|---|---|
| GUI 框架 | 复杂桌面布局、多个子窗口、分屏拖动、IME、快捷键、原生菜单/托盘、跨平台焦点行为 | 五平台能启动窗口；完成输入框/树/表格/分隔条/多窗口/拖放/IME smoke test |
| Terminal engine | VT 兼容、滚动回看、搜索、鼠标报告、字体与颜色、性能；libghostty-vt 的 Rust FFI/线程约束 | Rust 可稳定创建/销毁 terminal、写入/读取、resize、search；明确对象线程归属和 Send/Sync 约束 |
| PTY / ConPTY | 本地 shell、信号、窗口尺寸、进程退出和 Windows 行为差异 | Unix PTY 与 Windows ConPTY 分别完成 shell 启动、输入输出、resize、退出 |
| SSH channel 生命周期 | 一个连接上多 shell、SFTP、exec、转发并发；断线时统一取消 | 多 channel 压力/取消/断线测试，覆盖服务器 MaxSessions 限制和错误传播 |
| 凭据库 | 三个平台 API 不同，调用可能阻塞或弹系统授权 | get/set/delete、取消、无服务/锁定/权限拒绝、并发访问 |
| 打包与原生依赖 | GUI/终端库可能有系统动态库和签名/安装要求 | 五平台从干净 runner 生成可启动包，而不只是 cargo build |

### 当前结论
- 暂不拍板 GUI 框架。先用同一组验证用例比较候选，不根据“能编译”做决定。
- 终端核心与 UI 解耦；终端对象的线程归属先按单 owner thread/event loop 设计，待 FFI 实测后确定，不跨线程共享非线程安全对象。
- SSH client/channel、终端模拟器、UI 状态是三种不同生命周期，不能合并成一个 Session 巨型对象。
- Linux 服务器监控是远端 Linux 能力，不应错误地理解为本地客户端只支持 Linux。

## 6. 建议实施顺序

1. 规格冻结：把本表转成逐条验收项；补齐配置字段、同步字段、快捷键和各面板细节。
2. 技术 spike：GUI 框架对比；libghostty-vt Rust 集成；PTY/ConPTY；SSH 多 channel。
3. 领域与存储：domain、config-store、credential-store，先通过旧配置 fixture 和迁移测试。
4. SSH 纵向切片：单连接、host key、密码/密钥、保活、断线、重连。
5. 终端纵向切片：单终端、输入输出、resize、查找、编码、本地终端。
6. 多会话与布局：会话树、连接管理器、分屏、面板布局。
7. 文件与隧道：SFTP、传输队列、编辑器、-L/-R/-D。
8. 监控、导入、命令工具。
9. 云同步与体验完整性：格式互通测试、主题、地理位置、更新和打包。
10. 五平台验收：不仅编译；还要在原生 runner 上执行自动化测试和可启动 smoke test。

## 7. 本轮范围与下一步

本文件完成的是“功能域 → 目标职责”的第一轮映射。尚未完成：
- 逐个阅读 119 个 Go 测试文件，将其转换成行为验收目录；
- 对 app 中的状态与事件做更细的领域拆分；
- 根据 GUI/终端/SSH spike 结果确定最终 crate 数量和具体依赖版本；
- 建立配置和同步的跨语言 golden fixtures。

下一步先做 Go 测试行为目录 + 数据格式兼容规范。在这两项完成前，不创建一堆空 crate，也不开始大规模业务实现。
