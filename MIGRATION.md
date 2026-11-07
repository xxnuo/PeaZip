# Rust + nsh 迁移计划

目标是重写 PeaZip，交付 macOS、Linux、Windows、Android、iOS、Web；下面的阶段不是缩减最终范围。保留 Pascal 源码用于行为对照和上游跟进。未通过的平台不得标记已支持。

## 阶段与验收

| 阶段 | 工作 | 完成依据 | 状态 |
| --- | --- | --- | --- |
| 1 | Rust workspace、内存 ZIP 核心、安全边界、上游基线与 CI | 正常归档往返、恶意路径及限额测试；基线报告可执行 | 本地验证完成；远程 CI 待运行 |
| 2 | nsh Rust 界面：打开、列表、选择、创建、解压、错误反馈 | 用真实归档操作界面；UI 状态测试与本机启动验证 | 已有桌面/Web 与 Android ZIP 基本流程；更多格式、iOS 功能及剩余外部 API 验收待完成 |
| 3 | 安全文件系统与平台文件选择、后台任务、进度、取消、覆盖冲突 | 符号链接/竞态/失败清理测试；大文件不中断界面 | 已有有界读取、后台操作、新目标原子不覆盖发布、Web 创建取消与阶段进度；桌面取消/进度、其他冲突策略/流式大文件待完成 |
| 4 | 上游格式与功能逐项迁移 | 格式/操作能力清单；跨工具兼容样本、加密/分卷/损坏文件测试 | 已有 ZIP、TAR、TAR.GZ、7z 读取/选中解压；其余格式、加密/分卷等待完成 |
| 5 | 文件管理、搜索、预览、校验、转换、分割、配置、语言与无障碍 | 按上游功能回归；权限和交互验证 | 待完成 |
| 6 | 六平台构建、打包与发布 | 每个平台真实构建、运行、文件导入导出和归档往返；发布物可安装 | Web 浏览器流程、Android arm64 调试 APK 的导入/创建/保存/目录解压已验证；Linux GTK/X11 实际导入/解压已验证；iOS 构建与导出、Windows 实际运行及各平台发布验收待完成 |
| 7 | 持续跟进上游 | 定期差异报告、映射到回归测试、审查并更新基线 | 持续 |

## 工程边界

- `crates/peazip-core`：不依赖界面和平台文件系统，接受字节流；原生与 Web 共用归档语义。
- `crates/peazip-native`：原生文件系统适配；当前提供有界读取选中文件、临时文件写入与禁止覆盖发布。UI 只在桌面目标链接此 crate。
- nsh 源码 `/Volumes/MacData/Users/xxnuo/Projects/nsh` 只读。接入时固定 revision 并记录平台限制，禁止为了编译修改依赖源码。
- 文件系统提取必须单独处理链接、覆盖、原子发布和失败清理；内存路径校验不等价于安全落盘。
- 不支持的格式/操作必须明确反馈；不能把首个 ZIP 功能称为完整 PeaZip。
- 每个完整功能单独 commit。测试与实际结果一起更新状态，不能只根据编译通过判定产品完成。

## 上游跟进

`upstream-baseline.json` 固定已审查的 PeaZip 与 nsh revision。运行 `uv run python scripts/upstream_report.py` 输出相对本地上游引用的提交和文件差异；先用 `git fetch upstream` 更新引用。报告不会改源码或自动推进基线。

处理每次上游更新：审查差异 → 定位对应 Rust 功能（尚未迁移的加入阶段清单）→ 补回归样本/测试 → 实现并验证 → 更新基线并提交。基线仅表示已审查的来源，不能表示功能已全部迁移。共享分支更新使用 fetch + rebase，不覆盖他人提交。

CI 先验证三个桌面系统的核心测试与 Web 编译；这不等于 UI、移动平台或发布物已经验证。后续必须增加各平台实际运行与打包作业。

## nsh 已确认的接入约束

源码基线提供纯 Rust 的桌面入口和 CLI 生成的 Web/Android/iOS runner。复用 `examples/tasks-rs` 的组件和桌面宿主，关闭平台 crate 默认 features，避免引入不需要的 JS 引擎。

- Web 的 `blocking`/`background` 不可用，压缩需要 Worker 或可让出执行的方案；OPFS 不等价于原生路径 IO。
- Android 当前文件服务没有保存/目录对话框；需要 SAF 文档和目录授权适配。
- iOS picker 导入副本；导出与持久外部目录授权需要单独适配。
- 通用 picker 会整体读取文件，512 MiB 限额不能充当大归档方案。
- Linux 需 GTK 4.14+ 等系统开发库；移动端分别依赖 Android SDK/NDK/JDK 和 Xcode/签名。缺少工具链的构建列为未验证。

这些是待完成工程工作，不能因此删减平台目标。

## 验证记录

- 2026-11-07：核心 10 个集成测试通过，涵盖 Unicode/目录/空文件往返、重复路径与冲突、链接、路径穿越、各项资源限额、压缩炸弹、CRC、声明尺寸不实、建索引前的条目限额。核心 Clippy 与 wasm32 编译通过。
- 上游报告测试在临时 Git 仓库覆盖新增提交、无变化、无效引用、不相关历史；当前 PeaZip 上游与初始基线一致。
- 默认内存模式限归档 32 MiB、4096 条目、单文件 16 MiB、总解压 64 MiB、压缩比 200。后续流式大文件路径必须独立实现，不能宣称这些默认值已满足完整 PeaZip。
- ZIP 库固定 `=9.4.1`，因为有界目录迭代器目前位于 unstable API；升级必须重新验证安全测试。
- 2026-11-07：`cargo test --workspace --locked` 共 13 项通过；全 workspace Clippy、格式检查通过。UI library 与核心的 wasm32 编译通过。UI headless 流程覆盖打开真实 ZIP、取消、损坏/缺失文件恢复和大小限额。
- macOS `cargo run -p peazip-ui --locked` 启动成功，系统窗口列表确认进程创建了 840×632 的可见窗口；窗口截图失败，视觉与人工交互尚未验证。测试启动的进程已关闭。
- Web/Android/iOS 界面已接通通用文件 picker，桌面使用路径选择与后台读取。Web 当前在主线程解析有界目录；移动/Web picker 会先读文件，尚不能在读入前应用核心的 32 MiB 限额。浏览器与移动真机运行仍待验证。
- 远程 CI 已配置桌面核心/UI测试、Web编译和每周上游差异报告，但本轮未推送、未取得远程运行结果。

- 归档选择：按路径维护选择，虚拟列表使用可访问的 checkbox；全选、清空、取消/打开失败保留选择、成功打开归档重置选择的 headless 测试通过。UI 共 4 项测试；Clippy 与 wasm32 library 编译通过。
- 桌面创建：选择多个常规文件，以 basename 写入新 ZIP；后台处理并在保存后显示内容。取消任何对话框不写文件；成功创建重置选择，失败保留当前归档。现有目标（含链接/目录）不会覆盖。两个新 headless 测试验证真实内容往返、源文件不变、拒绝覆盖、取消和重试。
- 原生适配：拒绝末级链接和特殊文件，按单项/总量限额读取；同目录临时文件同步后使用 `persist_noclobber` 发布。源/目标父目录是用户选择的可信位置，尚未宣称防御父目录被恶意替换。Windows reparse 分支尚待 Windows 运行验证。
- 创建高重复数据时若 Deflate 超过读取压缩比限额，最多重试一次 Stored，读取恶意高压缩比归档仍拒绝；测试验证 1 MiB 全零数据和输出大小边界。
- 本轮集成验证：12 项 core、6 项 native、6 项 UI 测试全部通过；workspace Clippy、格式检查和 core/UI wasm32 编译通过。CI 已加入 native 在三个桌面系统上的测试。远程 CI、移动端和浏览器运行证据仍缺失。
- Windows 路径发布先规范化父目录，避免 tempfile 直接调用 Win32 时触发传统路径长度限制。`cargo check -p peazip-native --tests --locked --target x86_64-pc-windows-gnu` 通过，含 Windows 长路径测试的编译；这不代表该测试已经在 Windows 上运行。
- 选中解压：目录选择包含子项，重叠选择不会重复输出，未知条目报错；全包元数据仍先通过安全检查，仅解码所选载荷并校验 CRC。当前仍按全包声明大小限制资源，未声称支持超大归档的局部读取。
- 桌面解压：保存对话框选择新的输出目录名称，先在内存验证所选载荷，再写同父目录随机暂存目录；Unix 在创建时应用 0700。Linux/macOS/Windows 分别使用原子不覆盖重命名操作，已有文件、目录（含空目录）与链接都不会覆盖；发布成功后取消旧临时路径的析构清理。普通错误路径清理暂存，进程被终止或清理操作自身失败的残留恢复尚待实现。
- 界面持有打开/创建后的不可变 ZIP 快照，提取不会重开原路径；源文件被替换或删除后仍提取所见内容。当前操作期间禁用其他归档操作和选择；失败/取消保留选择以便重试。原生打开已复用安全常规文件读取，拒绝末级链接/FIFO等特殊文件，防止阻塞后台线程。
- 本轮验证：core 15 项、native 15 项、UI 8 项测试通过；包括 CRC 错误无输出、发布竞争、写入中途失败清理、部分/子目录解压、取消与快照一致性。全 workspace Clippy、Web library 编译通过；native 及测试在 Windows GNU、Linux GNU 目标交叉编译通过。跨平台运行、真实保存对话框及完整视觉验收仍未完成。
- 当前兼容边界：部分文件系统不接受的长单组件名、Unicode别名或 Windows 特殊保留名可能通过归档列表但提取失败；`create_new` 和原子发布防止覆盖，之后需补齐平台文件名冲突处理。Windows 暂存权限继承可信父目录 ACL，与 Unix 0700 不等同。

- 目录创建：native 接口支持文件/目录混选，保留根目录名与空目录，稳定排序归档条目；界面增加单独的文件夹选择入口，因为 nsh 的混合文件/目录选择仅部分平台支持。工具栏允许换行，窄窗口仍可操作解压按钮。
- 递归读取使用固定版本 `cap-std`/`cap-fs-ext` 4.0.3，子项相对已打开目录句柄并禁止跟随链接；目录被替换为链接的受控测试确认不会读到外部目标。按需打开子项、逐个处理选中根节点，避免同时持有全部文件或父目录句柄；在句柄上限 64 的子进程内归档 512 个选中文件通过。
- 递归资源限额在遍历/入队及读取时执行，空目录也计入条目上限；链接循环、特殊文件、重复根名、过长路径和超量内容均失败且不发布。输出放在源目录内时，与放在目录外的 ZIP 逐字节一致，不包含自身或暂存文件。
- 本轮验证：core 15、native 21、UI 10 个测试函数通过（低句柄限制子进程额外执行 1 次）；workspace Clippy/格式检查、UI wasm32 编译、native Windows/Linux 测试交叉编译均通过。UI 流程验证目录选择对话框类型、中文/空目录创建与解压、取消。尚无 Windows/Linux 运行时及浏览器实际运行证据。

- Web 宿主：`crates/peazip-web` 使用 nsh 的纯 Rust `rust_app!` 入口与 8 MiB 栈，复用现有 UI/core；未引入 TSX 或另一套界面。构建时从 Cargo 锁定的 nsh checkout 复制对应 JavaScript 宿主，不依赖本机只读源码路径。
- `uv run python scripts/build_web.py` 输出优化静态产物到 `target/web-dist`（约 12.5 MiB WASM，另含宿主 JS、HTML 和项目/nsh 许可证）；`--debug` 供开发，调试模块约 535 MiB，不作为交付产物。此目录与 Cargo 的 profile 缓存分离。可用 `uv run python -m http.server 8765 --bind 127.0.0.1 --directory target/web-dist` 本地运行。
- 浏览器验证：`pnpm --dir web install --frozen-lockfile`、`pnpm --dir web exec playwright install chromium`、`pnpm --dir web test`。真实 Chromium 加载优化 WASM，覆盖 Python ZIP 样本的中文名称/大小/空目录、单选/全选/清空、错误归档恢复、超出 32 MiB 的输入拒绝与空输入。没有 console/pageerror 异常；空输入不等同于已验证操作系统文件对话框的真实取消。已检查初始暗色界面及导入后的亮色截图。
- 新宿主通过 wasm32 Clippy，现有 workspace 测试、Clippy、格式检查和上游报告测试通过。CI 新增优化 Web 构建、真实 Chromium 测试及静态产物上传；尚未推送/运行远程 CI。本次仅本地验证，没有发布网站。

- Web 创建：浏览器选择多个文件，先检查数量、名称长度、16 MiB 单文件及 64 MiB 总量，再交给独立 Worker 读取与压缩，避免创建操作先把文件载荷读入 UI WASM。`peazip-worker` 使用与桌面相同的 Rust core，通过有界二进制协议再次检查输入；创建完成更新归档快照，用户点击 Download ZIP 发起下载，不宣称浏览器已经写入磁盘。
- Worker 在成功、错误或页面离开后释放；取消/错误保留原快照。回调绑定任务身份，避免旧请求结果完成新任务。缓存页面离开会结束 Rust pending；真实 bfcache 往返尚未验证，目前测试为连续两次合成 persisted pagehide 后可重新创建。
- 新构建工具要求：先运行 `cargo install wasm-bindgen-cli --version 0.2.130 --locked --root target/web-tools`，再运行 `uv run python scripts/build_web.py`。脚本检查 CLI 与 Cargo 锁定版本一致，使用官方胶水处理 Worker 依赖的 wasm-bindgen imports，不使用抛错占位导入；不会覆盖用户已有全局 CLI。优化 Worker 模块约 275 KiB，主界面约 12.5 MiB。
- 本轮验证：3 项 Worker 协议测试（截断/边界/UTF-8/重复/尾随字节/真实 ZIP 往返），现有 Rust workspace 回归及 native/wasm Clippy、格式检查通过。2 项真实 Chromium 流程通过，下载经 Python zipfile 独立验证后重开；取消及数量/大小超限不启动 Worker，重名失败恢复，忙时禁用按钮，7 个实际 Worker 全部释放。CI 增加 Worker 测试与匹配 CLI 安装。

- Web 创建任务新增 Cancel：仅创建进行时可操作，终止 Worker 并结束 Rust pending，不触发下载；保留原归档字节和选择。读取进度来自实际完成的文件数量（最多 100 次通知），随后显示压缩阶段，不宣称读取比例就是压缩比例。
- 进度通过 Rust 异步通道进入 nsh executor 后再更新 signal；不能在 JavaScript 调用的 WASM 回调内直接修改 signal，因为此时未进入 nsh reactive runtime。旧任务消息仍受身份检查保护。
- 本轮 4 项真实 Chromium 流程通过：WASM 加载阶段及 24 MiB 真实压缩阶段主动取消，Worker 终止且无成功输出，原 ZIP 字节/选择保持不变，同批文件重试成功；251 文件仅发出 84 条递增读取进度。现有 Rust workspace 测试、native/wasm Clippy、格式检查通过。

## Web 目录导出的边界与验证

浏览器 [getFileHandle/getDirectoryHandle 的 create](https://fs.spec.whatwg.org/#api-filesystemdirectoryhandle-getfilehandle) 不提供原子“不覆盖”，已有条目会返回其 handle；[目录 picker](https://wicg.github.io/file-system-access/#api-showdirectorypicker) 还要求用户激活和授权。不能复用桌面原子发布的保证描述 Web 实现。

现有实现使用用户授权父目录下的随机新子目录，保留所选目录树与空目录；先在 Worker 完成 core 校验与解压，任何已发现冲突立即停止。只有全部文件提交才显示完成；失败准确报告输出名、完成数、可能未完成路径及残留，不自动递归删除可能含外部新增文件的目录。并发外部修改时 API 无法提供严格 no-replace；单文件真实字节下载仍可作后续兼容补充，不能冒充完整目录导出。测试分别记录文件 API 行为、外部目录授权及失败状态，不能用 OPFS 测试替代全部平台验收。

- Worker 新增选中解压协议，直接返回经 CRC 校验的真实文件字节与目录条目，不重新打包 ZIP；支持目录子项选择、未知/恶意/截断/损坏输入拒绝。JS 写入前再次验证整个输出包及目录树预算。
- core 解压预算现在包含隐式父目录，防止少量条目路径生成超量目录；共享父目录去重，大小写不同的实际路径分别计数。该检查也用于桌面提取。
- Web 在用户操作期间请求目录授权，不支持目录 picker 时禁用提取并显示原因。提取使用已打开的归档快照，解压期间可终止 Worker；写入期间取消会请求 abort 并等待实际结果，close 已开始时不承诺撤销提交。取消/失败保留原归档与选择。
- 本轮验证：core 16、worker 6、native 21、UI 10 个测试函数及低句柄子进程通过；native/wasm Clippy、格式检查通过。JavaScript 32 项通过：9 项 Chromium 流程（含真实 Rust Worker/OPFS）和 23 项注入文件系统故障的单元测试。
- OPFS 实测包括真实解压字节、空目录、未选内容不导出、sentinel 保持不变、CRC 失败无输出、权限/取消恢复、能力缺失提示及加载期取消；写入期取消测试在真实 OPFS writer 外注入等待，验证 abort 和残留空文件报告。目录 picker 被测试替换为 OPFS 授权入口，**没有验证原生外部目录选择器、真实磁盘权限或外部修改竞态**，这些仍是待验收项。

## 移动宿主与构建验证

- Android 与 iOS 宿主复用同一 `peazip-ui`，固定 nsh revision 并关闭 JS 默认 features；移动 app ID 为 `io.peazip.rust`。根布局增加安全区域留白，避免按钮与状态栏/手势区重叠。
- `uv run python scripts/build_android.py` 使用锁定依赖中的 Java 源码，执行 Rust、javac、d8、aapt2、zipalign 和 apksigner；无需 Gradle，也不修改本机 nsh 源码。支持 `--sdk`、`--ndk`、`--jdk`、`--abi`；默认优化 Rust 构建，产物 `target/android/arm64-v8a/peazip-arm64-v8a-debug.apk` 为 **debug 签名/可调试预览**，不是商店发布包。
- 本机 Android 实测使用 SDK 35、build-tools 36.0.0、NDK 29.0.13113456、Android Studio 自带 JDK，进程内配置工具路径。APK 签名验证及 16 KiB zipalign 检查通过，ELF LOAD 段对齐为 0x4000。仅 arm64-v8a 已实际构建，其他 ABI 选项尚未验证。
- `uv run python scripts/smoke_android.py --serial emulator-5554 --apk target/android/arm64-v8a/peazip-arm64-v8a-debug.apk` 在独立 Android 36 模拟器上通过：实际安装/启动、系统 DocumentsUI 导入、中文名称和空目录、全选/清空及全部 checkbox 状态、取消 picker 保留内容、损坏归档报错后恢复。PID 保持稳定、无应用 AndroidRuntime fatal；最终 PNG/XML 在 `target/android-smoke`。脚本只删除自身随机测试文件；专用模拟器已关闭。当前只验证英文 API36 系统 picker，未验证真机和其他系统版本。
- `uv run python scripts/build_ios.py --target aarch64-apple-ios-sim` 已提供 SDK 查找、scoped Xcode、真实 Mach-O 架构/平台检查和 `.app` 打包；模拟器 ad-hoc 签名，真机输出明确未签名。**本机只有 Command Line Tools，Xcode 别名目标不存在，模拟器/真机 SDK 缺失，未生成或运行 iOS app**。5 项脚本测试验证环境边界和拒绝假平台产物，host stub 通过不能替代 iOS 编译。
- CI 新增 Android APK 与 iOS simulator app 构建/产物作业，尚未取得远程运行结果。Rust workspace 回归、Clippy、格式检查、6 项 Python 测试和 Web 32 项测试通过；只读 nsh 工作树保持干净。初始移动宿主仅提供导入、列表和选择；Android 后续功能见下，iOS 仍待 SDK 环境验证。

- Android 创建/保存：应用自有 `PeaZipActivity` 和 Rust JNI 适配复用 nsh 生命周期，不修改 nsh 源码。系统多选文件后先检查元数据数量/大小，再按实际字节有界读取；通过后台 Rust core 创建 ZIP，独立 Save ZIP 操作使用 `ACTION_CREATE_DOCUMENT`，关闭输出流成功才报告保存。
- 新输入桥只接受 content URI，未知或不可信大小仍受 16 MiB 单文件/64 MiB 总量限制；保存上限 32 MiB。AssetFileDescriptor 的 offset/length 是选中资产的边界，读取使用其输入流工厂；固定长度或非零 offset 的目标资产拒绝保存，防止越界或静默截断。保存失败报告可能残留，不自动删除用户文档。
- 大数组 JNI 转换与完成回调在后台执行，先清理 Activity 状态、释放传输名额，再完成 Rust future；generation/request ID 隔离旧回调。销毁时使用 CancellationSignal 并异步关闭跟踪资源，全局有界传输/取消/关闭执行器阻止资源累计。若 provider 的取消和关闭均不返回，唯一名额会持续占用、拒绝新创建/保存请求，需进程重启恢复；不声称能强制终止不合作的 provider。
- `uv run python scripts/check_android_bridge.py` 已加入 Android CI：直接引用生产 Rust/Java，运行 11 项有界读、14 项真实 JVM/JNI、7 项资产边界检查，以及 64 MiB 后台转换、旧任务清理期间拒绝 200 次新请求等生命周期检查。部分 Android 类为测试 doubles，不替代设备验证；这些检查独立于桌面 cargo test，避免强制桌面开发安装 JDK。
- 最终 APK 在 Android 36 arm64 模拟器通过扩展 smoke：多选中文名/二进制/空文件创建，17 MiB 输入拒绝且保留旧状态，取消保存不写目标，同名目标由当前 Documents provider 自动生成 `(1)`、sentinel 未覆盖；拉回 ZIP 用 Python zipfile 独立核对名称、内容和 CRC，重新导入成功。PID 稳定、无应用 fatal；新标题宽度与列间距已在长文件名截图检查。
- 本轮 Rust workspace 回归、主机及 Android target Clippy、格式检查、Web 32 项测试通过。专用模拟器已关闭，未部署或上传发布包。只实测本机 Documents provider，未宣称所有云 provider、保存失败和真实恶意 provider 场景都已设备验收。

- Android 目录解压：先在 Rust 后台完成所选 payload/CRC 校验，再通过 `ACTION_OPEN_DOCUMENT_TREE` 授权父目录；以 DocumentsContract 的 opaque ID 创建随机新子目录，保留所选子树、隐式父目录和空目录。Java 输出包再次校验路径、类型、重名、Unicode alias 和资源预算；创建返回的 authority/tree scope、名称、类型和 ID 均检查，普通文件 MIME 允许 provider 规范化。
- 目录查询的 Loading/非空 EXTRA_ERROR 不得作为“不存在”的依据；单字段与累计 provider 元数据也有额度，URI 按需构造，避免长 tree ID 在每行重复放大。任何已发现冲突、重命名或 ID alias 都停止；不会合并已有目录或自动删除残留。SAF 没有跨 provider/外部并发修改的原子 no-replace 保证，这不等同于桌面原子发布。
- 读写 helper 在关闭前记录 reliable descriptor 能力，关闭后检查缓存的远端错误；已可检测的 provider 失败不会再计入成功。仅能检测本端关闭时已报告的错误，不等同于云端事务提交确认。该修复同时覆盖 ZIP 保存与目录写入。
- `uv run python scripts/check_android_device.py --serial emulator-5554 --apk target/android/arm64-v8a/peazip-arm64-v8a-debug.apk` 构建独立 instrumentation APK，运行后卸载测试包，生产入口没有测试后门。本机通过 7 项真实 Android reliable-pipe/普通文件检查，包括“普通 close 成功但远端已报错”场景。
- 本轮 hostchecks：86 项导出策略、25 项 JNI、13 项资产、11 项读取及生命周期检查通过；这些使用部分 Android doubles。最终 APK 全量真实 Android 36 DocumentsUI/SAF smoke 退出码 0：树 picker 取消无输出，CRC 错误在 picker 前拒绝；嵌套二进制和 txt 内容、空目录、未选文件缺席、sentinel 不变及错误后恢复均已拉回核对。最终本机运行 token 为 `cf3a77e3610d`，截图在 `target/android-smoke/<token>/extracted.png`。
- Rust workspace 回归、主机/Android target Clippy、格式检查和 Web 32 项测试通过；只读 nsh 保持未改动。Android 此处验证的是 API36 本机 Documents provider，不代表所有云 provider、真实外部竞态或进程被强杀后的恢复都完成。

## 格式后端推进

- 新增内容识别和通用 `list_archive` / `extract_archive` / `extract_selected_archive`，保留 ZIP 专用 API。未压缩 TAR 复用路径、重名/父子冲突、实际字节及隐式目录预算；支持 ustar、常见 PAX 本地 path/size、GNU 长名、UTF-8、空目录，去除标准前导 `./` 并省略根目录条目。
- 使用锁定的 tar 0.4.46 原始头读取，不在限额前展开 GNU/PAX/sparse 内容；扩展头和根目录计入扫描预算，扩展数据计入字节预算。TAR 头有 checksum，**普通 payload 没有 CRC**，提取成功不能证明其内容未被修改。
- 明确拒绝链接、特殊/稀疏文件（含 GNU、SCHILY、Solaris sparse 扩展），以及当前未实现的全局 path/size、重复 path/size 覆盖等变体。要求完整结束块、仅零尾部，不能将隐藏条目/拼接容器当作成功；这是严格兼容策略，部分标准工具可接受的尾部也会被拒绝。此阶段不读取压缩 TAR（后续 GZIP 支持见下），也不创建新 TAR。
- 有效首部 TAR 优先于 ZIP magic/SFX 探测，防止 PAX 头和 ZIP 尾部混合时隐藏 TAR 内容；无效 TAR checksum 不误拒普通 ZIP/MZ SFX。PAX 关键词长度和控制字符也受检查，避免未知扩展将巨大输入放大到错误消息。
- UI 打开/提取采用通用接口；Web Worker、桌面原子目录发布和 Android SAF 复用同一解码结果。Web/Android 保存原归档按识别出的类型使用 `.zip` 或 `.tar` 与对应 MIME，不重新打包、不会把 TAR 标成 ZIP；创建操作仍只生成 ZIP。
- 本轮验证：core 40 项测试，新增 native、headless UI、worker TAR 流程通过；系统 tar 与 Python tarfile 验证参考样本。浏览器 33 项通过，包含误命名 `.zip` 的 TAR 正确识别、下载字节不变和 OPFS 选中提取。Android 完整 ZIP+TAR smoke 已通过：真实 application/x-tar 保存/取消、字节完全一致及独立 tarfile 校验、真实 SAF 子树/Unicode/二进制/空目录、未选内容缺席、既有输出不变。设备记录 `target/android-smoke/be58e81d781d/console.log`。
- 主机 JNI 检查已覆盖 ZIP/TAR 格式传输、非法代码拒绝和生命周期；格式与结构错误统一使用 archive 文案。Rust 回归/Clippy、格式检查保持通过。iOS 当前配置仍未发现完整 SDK；Docker 指向本地 OrbStack socket，但 daemon 未运行，未据此宣称 Linux 运行验收完成。

- 新增 TAR.GZ / TGZ 内容识别与读取，复用已有纯 Rust flate2/zlib-rs；创建仍只支持 ZIP，不将普通单文件 GZIP 误认为 TAR。多个 gzip 成员作为同一 TAR 字节流连接，支持跨头部/文件边界；整个流的 CRC、ISIZE、可选 FHCRC 和尾部均验证后才返回列表或选中内容。
- 解码前检查 32 MiB 输入上限，输出按总内容与 TAR 头部/对齐预算约束；GZIP 头单个最多 64 KiB、累计 1 MiB，成员最多 4096，不按 ISIZE 分配。压缩比按实际 DEFLATE 字节检查，空成员不增加预算；固定 10 KiB 余量允许标准空 TAR 的结束记录，不能无限放大。严格 TAR 路径/特殊文件/结束块策略保持有效。
- `list_archive_with_format` 一次验证返回格式与列表，避免主机重复解压。Web 打开改由真实 Rust Worker 执行解码，返回有界元数据；主线程不再次探测 GZIP。列表阶段支持 Cancel，错误或取消保留原快照和选择，结束后释放 Worker；文件 picker 读取阶段仍受 nsh 自身限制。
- Web 下载与 Android 保存使用 `Archive.tar.gz` / `application/gzip` 并保留原字节，文件筛选支持 `.gz`、`.tgz`；原 TAR/ZIP 的格式与保存语义保持一致。

- 本轮 Rust workspace 103 次测试执行通过（含低句柄子进程重复），core 55 项；主机/wasm/Android target Clippy 与格式检查通过。浏览器 36 项通过，覆盖真实列表 Worker、TAR.GZ MIME/原字节下载、CRC 失败保留快照、OPFS 子树提取和 Cancel/pagehide 后重试及旧回调隔离。pagehide 仍为合成事件，OPFS 仍不替代外部目录权限验收。主机 Android 检查包括 27 项 JNI、86 项导出策略、13 项资产、11 项有界读取及生命周期，覆盖三种保存格式。

- Android 36 全量 ZIP/TAR/TAR.GZ smoke 通过，记录 `target/android-smoke/016b44ede8fc`；最终 APK 重建后 TAR.GZ 定向复验通过，记录 `target/android-smoke/e4a54df9cfd4`。真实 `application/gzip` 保存/取消、原字节和独立 gzip/tarfile 核对、SAF 子树提取、空目录和未选内容缺席、CRC 拒绝前后快照/选择/既有输出均已检查，PID 稳定且无应用 fatal。专用模拟器测试文件已清理；仍仅覆盖本机 Documents provider，未新增 iOS 或 Windows/Linux 实际运行证据。

- 7z 后端采用固定 `sevenz-turbo = 0.29.5`，关闭默认 features，锁定纯 Rust LZMA/CRC 依赖；最低 Rust 版本提升为 1.97.1。使用其 [ArchiveLimits](https://docs.rs/sevenz-turbo/0.29.5/sevenz_turbo/struct.ArchiveLimits.html) 在头部/名称/对象分配和 decoder 构造前限制资源，不使用默认无限预算构造；字典和模型的估算预算不等于整个应用的 heap 上限。后续升级必须重跑恶意头部、计数、solid 和实际浏览器测试。
- 本阶段读取普通及 solid 7z，支持 COPY/LZMA/LZMA2 与已启用过滤器；复用路径冲突、特殊文件拒绝、选择子目录和原子/授权目录输出。元数据先整体校验，再完整消费选中内容所属 block（包括其中未选成员），等待 block 校验成功后交付结果；未涉及 block 的内容 CRC 不在局部提取时验证，列表不宣称验证 payload。数据块禁用后端会提前返回 EOF 的 CRC 包装，由 core 对实际读取逐成员/逐块计算 CRC，同时保留后端流末尾检查；防止声明前缀 CRC 掩盖 LZMA2 多余输出。
- 只接受完整、头部和 packed ranges 连续的非 SFX 单文件容器；拒绝恢复猜测、附加尾部、加密、anti-item、链接/reparse/特殊类型及未启用 codec。创建仍仅 ZIP，不宣称已完成 7z 加密、分卷、更新或写入。Web/Android 保存使用 `.7z` 和 `application/x-7z-compressed`，保留输入字节；新依赖的许可证和贡献者声明随 Web/Android/iOS 打包脚本复制。

- 7z 完整性边界：校验外层 start/next header、被读取成员和块的已声明内容 CRC；CRC 可缺省，不能作为真实性证明。锁定后端尚不核验 packed-stream CRC，也不对压缩头声明长度之后追加 EOF 探测；不据此宣称覆盖所有畸形编码的完整性。特殊零长度 stream block 在入口拒绝，普通无 stream 空文件/目录继续支持。

- 本轮新增 18 项 7z 核心测试，core 合计 73 项，工作区 125 次执行通过。样本由系统 7z 独立生成，覆盖 COPY/LZMA/LZMA2、solid/non-solid、压缩头、BCJ/BCJ2/Delta、中文/空目录、链接/特殊属性拒绝、头部 CRC/范围/计数/名称/字典限制和选择子树；构造 LZMA2 前缀 CRC 样本确认多余输出不再被 CRC 包装掩盖，未选 solid 内容损坏不会交付部分结果。主机、WASM、Android target Clippy 与格式检查通过。iOS 真机/模拟器、Windows、Linux 的核心交叉编译通过；这些仅为编译证据。

- 最终 Web 优化产物的 37 项浏览器/文件导出测试通过：真实 7z 列表与提取 Worker、内容识别、原字节及 MIME 下载、头 CRC/next-header 失败保留快照，OPFS 输出按系统 7z 样本核对。Android 主机桥接检查覆盖 28 项 JNI 与 ZIP/TAR/TAR.GZ/7Z 保存类型，所有原生命周期/有界 I/O 检查通过。

- Linux 实际运行：启动本机 OrbStack，使用独立 Ubuntu 24.04 arm64 容器、GTK 4.14.5、GStreamer 1.24.2、Rust 1.99.1，源码只读挂载。最终 core/native/worker/UI 共 125 次测试执行通过；真实 GTK 应用在 Xvfb+DBus 下连续两次经文件选择器导入 7z、选择 docs/ 并提取，系统 7z 字节对照、空目录、未选内容缺席和哨兵不变均通过；中文字体和界面截图已检查。证据位于 `target/linux-runtime-20261108-a/final-test.log`、`final-smoke1.log`、`final-smoke2.log`。新增 `bash scripts/smoke_linux.sh target/debug/peazip-ui target/linux-smoke` 和 Linux CI 步骤；远程 CI 尚未运行。该脚本针对 GTK 4.14 英文 X11 固定布局，不代表 Wayland、门户或所有桌面环境；容器已停止，保留可复用缓存。

- Android 最终 APK 7z 真实 DocumentsUI/SAF smoke 通过，记录 `target/android-smoke/a5f82157f576`：打开 6 项、`application/x-7z-compressed` 保存/取消、保存字节与独立系统 7z 校验一致，选中 docs/ 提取 5 项、中文/二进制/空目录正确、未选文件缺席、既有哨兵未改；损坏头 CRC 在写出前拒绝并保留旧快照/选择。PID 稳定无应用 fatal，测试文件已清理，专用模拟器已关闭。

下一步：补充更多归档格式与大文件流式处理，同时继续推进 iOS SDK/文件导出和 Windows/Linux 实际运行；补齐 Web 外部目录、桌面/移动端主动取消与进度、冲突策略及文件管理功能。Web/Android 打开归档仍走 nsh picker，创建使用独立有界入口；两者限制不可混为一谈。六平台实际运行、完整功能与发布验收仍是最终要求。

## 归档完整性测试

- 新增 `test_archive` 和各端 Test archive 操作：检查当前不可变快照中的全部条目，不受选择范围影响，不打开保存/目录对话框，也不写出文件。ZIP/7z 使用固定缓冲读取，7z 完整检查所有 block 与成员 CRC；TAR.GZ 保留已有有界整包解码，避免再复制各文件 payload。
- 报告明确区分条目、文件、实际内容字节和带内容校验和的文件数量。TAR 仅验证结构且覆盖数为零，GZIP 容器 CRC 覆盖全部文件；7z 只有实际存在成员或所属数据块 CRC 的文件计入，空文件不虚增覆盖。不宣称校验和证明真实性，也不扩大已记录的后端完整性保证。
- Web 使用真实独立 Worker，只返回 20 字节有界报告；支持 Cancel/pagehide、重试和旧回调隔离。桌面和 Android 使用后台任务；成功、失败、取消均保留原快照和选择。桌面/移动端此操作的主动取消仍待接入。
- 验证：core 82 项、macOS 工作区 139 次测试执行通过，主机/WASM/Android Clippy 与格式检查通过；16 MiB ZIP/7z 样本的最大单次分配小于 4 MiB，确认未保留整份文件输出。浏览器 41 项通过，包括四格式校验覆盖、未选文件损坏、只读操作和实际 Worker 取消/旧回调。Python 6 项回归通过。
- Android 36 最终 APK 定向 smoke 通过，记录 `target/android-smoke/8204e9cb53b6`：ZIP/TAR/TAR.GZ/7z 报告精确，未选 ZIP 文件 CRC 损坏被拒绝，快照/选择/Downloads/Documents/既有输出保持不变，无系统保存对话框，PID 稳定且无应用 fatal；测试文件已清理，专用模拟器已关闭。
- Linux GTK/X11 测试脚本新增实际点击 Test archive、OCR 核对报告与选择、无输出及哨兵检查，然后继续提取并比较真实字节；连续两轮通过，证据位于 `target/linux-runtime-20261108-a/test-archive-final1.log`、`test-archive-final2.log` 与 `test-archive-final-tests.log`。CI 加入 tesseract-ocr 依赖，仍未运行远程 CI；GTK 固定布局与 X11 验证范围不变。

## 原版视觉基线与归档导航

- 原版基线已采集：官方 PeaZip 11.4.0 Linux ARM64 GTK2 portable，与固定源码版本号一致；下载资产 SHA256 与官方发布值一致。`tests/ui-baseline/peazip-11.4.0-linux-gtk2/` 保留 1024×600、1440×900 的空文件夹、sample.7z 根目录与 docs 子目录共六张真实截图，以及版本、来源、采集步骤和哈希。版本一致不等于证明发布二进制由精确固定 commit 构建；GTK2 原版与 GTK4 新版的平台差异单独记录。
- 归档视图默认按目录层级浏览，支持隐式父目录、Back/Forward/Up/Archive root、路径输入和 Go、目录优先的名称/大小双向排序；All entries 可切回全部真实条目的平铺视图。历史最多 256 项，成功打开/创建重置历史与选择，重新打开相同归档也清除旧的无效路径草稿和错误。
- 宽屏侧栏和主列表均按当前层级生成并虚拟化；窄屏隐藏侧栏与 Type 列，保留导航和操作。复用原版 main/main-dark 的操作与导航 PNG 图标（未改像素，嵌入 data URI），普通文件使用简单纸张轮廓占位；原 `[ ]` 文本选择标记替换为方框、部分选择指示及行高亮。名称、类型和归档路径限制单行，完整路径仍作为控件可访问名称；长中文文件名及扩展名的窄屏实际渲染测试通过，无页面横向溢出。
- 选择语义反映实际提取范围：选中父目录时后代显示已选，取消子项会展开祖先选择为剩余真实文件/空目录，避免 core 再次包含被取消内容。隐式目录只映射到真实后代条目，不能向 core 传递不存在的路径。新增资源回归确认深层路径不会一次生成数万个目录节点。
- 本轮 macOS 工作区 152 次测试执行通过，UI 包含 11 项模型测试和 17 项 headless 流程；父目录选中后取消子文件已验证实际落盘排除。浏览器 56 项通过，覆盖导航、排序、非法路径、重开状态、实际 OPFS 排除输出，以及 1024×600、1440×900、360×800 下亮暗主题。截图为 `target/web-folder-*.png`；生产源码摘要和构建命令记录在 `target/ui-navigation-evidence.json`。
- Linux 最终源码哈希核对一致，152 次测试执行与两轮真实 GTK 导航/Test/解压通过；六张新版对照截图、验证元数据在 `target/linux-ui-rewrite/`。原版与新版同尺寸/同 sample.7z 场景已人工对照；像素差异只记录现状，不充当自动通过阈值。空状态原版是空文件夹，新版是未打开归档，不能据此宣称本地文件系统视图已迁移。脚本改用 OCR 定位主要控件，保留真实文件选择与字节对照；只覆盖已记录的 GTK/X11 环境。
- 路径编辑使用字面输入（不自动大写或纠正）；合法 Enter/Go、历史导航与重开归档后显式结束编辑，避免 Android 首次目录点击只失焦而未选择。非法 Enter 保留编辑焦点，便于修正。
- Android 最终 APK 的完整导航 smoke 退出码 0，记录 `target/android-smoke/a724445945d9`；真实 DocumentsUI/SAF 验证隐式目录仅导出实际后代、父目录选中后取消子文件确实不输出该文件、空目录/其他输出/哨兵正确，第一次点击生效，路径 Enter 成功失焦、非法输入保留焦点，重开与视图偏好正确。独立输入复验记录 `target/android-smoke/fd0c358f069f`。新增可重复运行的 `uv run python scripts/smoke_android_navigation.py --serial emulator-5554 --apk target/android/arm64-v8a/peazip-arm64-v8a-debug.apk`；测试文件清理、PID 稳定无应用 fatal，专用模拟器已关闭。
- 阶段 2a 仍未完成：原版完整菜单、文件系统/书签/历史导航、双击与快捷键、列宽调整及完整属性列、创建/解压选项对话框、跨平台窗口/主题和 iOS/Windows 实际运行均需继续验收。此提交是可用的归档导航单元，不将它或六张截图等同于完整 PeaZip 界面迁移。
