# 原版测试行为目录

> Draft 1  
> 来源：原版 finallyshell @ 78c2f91603bfb014b800a88cfb68fded64934141  
> 盘点范围：119 个 *_test.go 文件。此文是测试主题目录，不声称逐条复刻了每个测试断言；迁移时还要将原测试代码和 fixture 转成 Rust 测试。

## 1. 测试分布

| 子系统 | 测试文件数 | 主要验收面 |
|---|---:|---|
| internal/app | 67 | 用例编排、会话状态、窗口/布局、各功能页和 UI 行为 |
| internal/cloudsync | 7 | 加密、三方合并、Gist/WebDAV/S3、冲突与重试 |
| internal/config | 10 | 配置、迁移、字段往返、历史、快捷命令、同步白名单 |
| internal/importer | 7 | 各来源解析、编码、异常输入、导入问题 |
| internal/monitor | 11 | 远端脚本解析、进程/socket、shell cwd、路由追踪 |
| internal/remotefs | 4 | SFTP、文件操作、传输取消、归档安全、文本编辑 |
| internal/sshx | 4 | SSH Agent、转发、保活、代理和跳板 |
| internal/fileicons | 1 | 主题解析、匹配、字体、安装、扫描编辑器扩展 |
| internal/geo | 2 | 数据下载/回退/取消、查询与缓存 |
| internal/termenc | 1 | 编码转换、resize 与鼠标报告透传 |
| internal/icons | 1 | agent 图标资源 |
| 根目录/工具 | 4 | 渲染平台 workaround、托盘图标、更新配置、natives 重试、release 工具（实际文件数以 tree 为准） |

说明：上表按目录归类；根目录/工具项包含 rendering、tray、updates、tools/natives、tools/release 的测试文件。统计汇总应以 Git tree 的实际文件路径为准。

## 2. 按领域的行为验收

### 2.1 配置、持久化与凭据
原版测试主题：
- Settings/Host 的默认值、字段 round-trip、Host clone、大小写兼容字段。
- hosts v1 迁移、坏文件备份、无文件启动、文件夹、最近连接、删除状态。
- 未知字段保留；Proxy/Tunnel 未知键保留。
- 快捷命令无文件、读写往返、损坏/不可读文件、未知字段、更高版本兼容、命令多行语义。
- 历史去重、排序、容量裁剪、shell 历史解析、主机路径隔离。
- 同步设置白名单、设置往返、icon theme 同步例外、本地 sync-state。
- 凭据记住/不记住、错误密码重新询问、passphrase、代理/跳板凭据、主机删除清理、复制连接复制凭据、凭据不可用、后台线程访问、错误日志去重。

Rust 迁移测试必须独立于 UI：配置 codec、迁移、文件原子写入、未知字段、凭据端口分别测试。

### 2.2 SSH、认证与隧道
原版测试主题：
- 密码/私钥/SSH Agent 登录、Agent 多 key 尝试、Agent 默认地址和 key 列表。
- Agent 在跳板机上的认证、Agent 错误。
- SOCKS5、HTTP CONNECT、跳板机连接、跳板链错误归因。
- keepalive 间隔、关闭 keepalive、空闲会话检测、跳板机逐跳 keepalive。
- 本地/远程/动态转发的建立与关闭；通过跳板机的隧道。
- UI 侧测试还覆盖首次连接、连接错误、缺失凭据、连接中取消、主机代理设置编辑。

Rust 迁移必须有进程内 SSH 测试服务器，覆盖多 shell channel、SFTP/exec/forward 并行、断线时取消、MaxSessions 限制、host key 信任状态。不要只用 mock 验证协议行为。

### 2.3 终端、编码与本地 shell
原版测试主题：
- 远端编码 UTF-8 passthrough、GBK/GB18030/Big5 等解码和编码。
- resize 和鼠标报告不应被字符转码破坏。
- TERM、Backspace/Delete、快捷键重绑定、平台默认快捷键、Option-as-Alt。
- 本地终端启动、命令发送、退出、重新连接/结束状态、面板支持范围、发送目标。
- 终端查找、全屏、查找栏焦点和快捷键。
- 分屏树插入/移除/交换、焦点、拖动、关闭、重连、尺寸调整和越界值处理。
- 分屏移动后仍跟随原来的 shell；每个 pane 的 cwd/前台程序读取与 shell 对应。

Rust 终端测试分为 terminal-core 的纯状态机测试、terminal-session 的 channel/PTY 集成测试、UI 的交互测试。libghostty-vt 行为需以独立 spike 验证。

### 2.4 会话、连接树与布局
原版测试主题：
- 新标签快速连接、最近连接排序、空配置状态、连接后选中策略。
- 会话重命名/取消/恢复原名/F2/重连后保留、不同 UI 标签引用名称。
- 连接树搜索、文件夹展开状态、菜单、快捷键、连接页与连接管理器状态同步。
- 连接管理器搜索、文件夹增删改移、软删除/恢复/永久删除、复制/重命名、跳板机引用。
- 批量连接的密码询问和取消。
- 面板隐藏/恢复、左右侧切换、监控和面板同列、面板行共享连接和跟随会话移动。
- 子窗口每类只开一个、父子模态关系、Esc/Cmd+W、关闭主窗口时关闭子窗口。
- 关闭会话时选择项保持合理；退出确认和托盘行为。

### 2.5 文件系统、传输与远程编辑
原版测试主题：
- SFTP 目录列表排序、软链接目标/断链/循环、目录链接不递归。
- 上传/下载文件和目录、取消、路径包含判断、重命名、新建、递归删除。
- 归档/解包脚本、shell quoting、无 exec 降级、流中断、取消。
- 解包路径穿越拒绝、不能跟随链接、硬链接/符号链接限制、Windows 文件名、损坏 gzip trailer。
- 远程文本读取、二进制/大文件识别、编码转换、保存替换/原地保存。
- UI 文件编辑冲突检测、未保存修改提示、断线处理、文件操作菜单、传输队列和重试。

安全测试必须优先：路径穿越、符号链接逃逸、硬链接、归档炸弹/超大输入、远端内容变更覆盖、取消后残留文件。

### 2.6 监控、进程、网络与地理位置
原版测试主题：
- Linux stats 脚本解析：uptime/load/CPU/mem/swap/net/disks/procs。
- 网卡速率增量、历史长度、接口选择、多个接口汇总。
- procps 与 BusyBox ps 解析、缺字段、CPU delta、进程详情、信号脚本和退出状态。
- ss 新旧格式与 /proc/net fallback；非 Linux 能力提示。
- shell cwd/前台程序解析、多个 pane 对应 shell、SSH 无法读取时降级。
- 系统信息脚本缺少 lscpu/nproc/ip 等工具时的 fallback。
- traceroute/tracepath/NextTrace 解析和目标地址校验。
- agent 名称精确匹配和运行时/包管理器启动器识别。
- ip2region 数据下载来源 fallback、失败、取消、慢下载、缓存、LAN 地址和在线查询。

这些能力中多数针对远端 Linux 主机，不应与本地桌面平台矩阵混为一谈。

### 2.7 导入器
原版测试主题：
- FinalShell 配置目录发现、conn 文件和导出文件读取、缺失/损坏输入。
- Xshell session folder、.xsh 和 .xts 导出布局。
- SecureCRT ini、sessions 文件夹、配置发现、key 文件。
- MobaXterm ini、bookmark/session/export、GBK 文本、外置 key。
- OpenSSH config Host 块、通配符、Include、变量展开、ProxyJump。
- CSV/XLSX 列识别、表头映射、模板、数字文本、密码空格、XLSX 限制。
- UTF-8/UTF-16/GB18030 解码、导入问题、密钥文件与密码、去重和文件夹归属。

导入器必须使用固定 fixture；不能依赖开发者机器上安装了其他客户端。

### 2.8 云同步
原版测试主题：
- Seal/Open、salt 复用与 nonce 更新、密文篡改、非法 envelope 拒绝。
- 三方合并：主机/文件夹/设置/秘密的新增、修改、删除、双边修改、无 base、顺序无关比较。
- Gist 分页发现、冲突、缺失时创建、认证、截断响应、Link header。
- WebDAV 文件夹创建、冲突、认证、缺少 ETag。
- S3 SigV4 标准向量、路径转义、对象 URL、认证、条件写冲突与重试。
- 双设备同步、错误密码、远端冲突重试、排除 secrets、新 schema 拒绝。
- UI 同步期间本地改动、目的地变化、保存密码变化和自动同步。

### 2.9 UI 视觉与资源
原版测试主题：
- 中英文布局和文案、主题、字体、颜色、背景图片、EXIF 方向和尺寸限制。
- 后台读取不阻塞 UI；旧请求结果不能覆盖新状态。
- 图标主题 JSONC/VSIX/扩展目录、文件/文件夹/语言匹配、暗色主题、字体转换。
- 快捷键冲突、平台名称差异、快捷键录制和持久化。
- 关键页面截图作为布局回归测试。

## 3. 测试迁移分层

| 层级 | 对应内容 | 是否要求五平台 |
|---|---|---|
| L0 纯单元测试 | parser、merge、codec、domain state、path rules、key mapping | 是，所有 runner 执行 |
| L1 本地集成测试 | 临时目录、mock HTTP、进程内 SSH server、SFTP fixture | 是，能运行的 runner 全执行 |
| L2 平台集成测试 | Keychain/Credential Manager/Secret Service、PTY/ConPTY、tray/window | 原生平台 runner 分别执行 |
| L3 UI 行为/截图 | 分屏、拖放、窗口、IME、布局、快捷键 | 需要真实桌面会话；CI 无桌面时至少构建，另设桌面 smoke |
| L4 真实服务/网络测试 | 公网 SSH、真实 Gist/WebDAV/S3、在线地理查询 | 不作为每次 PR 必跑；用可控 mock 和定期 nightly 验证 |

## 4. Rust 测试落地规则

- 每个原版测试主题转为一个可读的 Rust 测试名；不要求照搬 Go 函数名，但要能追溯。
- 用表驱动测试覆盖枚举和边界，不把多个失败原因塞进一个“happy path”。
- 协议/加密兼容使用 Go 与 Rust 双向 golden fixture；只有 Rust 自己加解密成功不足以证明兼容。
- 远端脚本测试保留 BusyBox/procps、工具缺失、权限不足和非 Linux 分支。
- UI 测试按用户可见行为描述，不能只检查 widget 存在。
- 对异步任务增加“过期结果被丢弃”“取消后不回调/不写回”“关闭后无悬挂任务”测试。
- 对资源读取增加损坏输入、超大输入、路径逃逸和权限错误测试。

## 5. 下一步

下一阶段不是直接把 119 个 Go 测试文件逐行翻译，而是：
1. 从 config/cloudsync/sshx/termenc/remotefs 中挑选纯核心契约，先制作跨语言 golden fixtures。
2. 对 libghostty-vt 与 GUI 框架做短期技术 spike，确定终端线程归属和 UI 事件模型。
3. 再开始第一个 Rust 纵向切片：config-store + domain 的 Host/Folder 读写迁移。
