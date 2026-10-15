# Nova Launcher

Minecraft 启动器 MVP（Tauri 2 + Rust + TypeScript），淡蓝色主题。

## 界面

左侧窄图标栏切换三个页面：

- **主页**：玩家名 + 启动按钮
- **设置**：玩家名、Java 可执行文件（浏览/检测）、macOS 本机 Java 自动寻找与选用、内存分配
- **日志**：启用日志采集开关（下次启动生效）、导出日志按钮、实时日志

## 启动逻辑

- `.minecraft` 目录不做设置；macOS 打包应用自动识别 **`Nova Launcher.app` 所在目录**下的 `.minecraft`，不依赖 Finder / Terminal 的工作目录。其他平台及未打包的开发模式仍使用进程工作目录。
- 版本自动选择：`versions/` 下 jar 修改时间最新的版本
- classpath：按版本 JSON 声明选择存在的非 native 依赖，无 JSON 时才递归收集 `libraries/**/*.jar`，最后追加版本 jar；natives 取 `.minecraft/natives`（或版本目录内 `*natives*`）。Apple Silicon macOS 使用下方专用逻辑。
- 主类 / tweakClass 自动从版本 json 解析（如 Forge 的 `net.minecraft.launchwrapper.Launch` + `FMLTweaker`），无 json 时按版本名推断
- 离线用户名（空格转 `_`），配置持久化到与 `.minecraft` 同级的 `nova_launcher_config.json`（便携，不写系统应用配置目录）

暂不支持：正版登录、版本隔离；Java 自动寻找目前仅实现 macOS，其他平台仍可手动浏览、填写路径和检测。

### Apple Silicon macOS 启动

ARM 原生启动器及 Apple Silicon 上经 Rosetta 运行的 Intel 启动器，均按 `mac-dlls/launch-modpack-arm64.py` 的 LWJGL 2 启动方式构造命令。Intel Mac、Windows 和 Linux 保持原启动逻辑。

- 仍自动选择版本目录，不硬编码 `1.6.4`；必须有可读取的版本 JSON，从中读取 `mainClass`、`id` 和 `libraries`。
- 按 JSON 顺序保留非 native 依赖，不应用旧版 OS rules；移除所有 `org.lwjgl.lwjgl` 依赖（包括 nightly 和工具类），不把 native jar 放入 classpath。
- 将 `net.java.jinput:jinput`、`net.java.jutils:jutils` 分别替换为 `mac-dlls/dist/jinput-2.0.5.jar`、`jutils-1.0.0.jar`，最后追加该目录的 `lwjgl.jar`、`lwjgl_util.jar` 和原版本 jar。
- `java.library.path`、`org.lwjgl.librarypath`、`net.java.games.input.librarypath` 均指向整合包同级的 `mac-dlls/dist/natives/`，不回退到旧的 `.minecraft/natives`。
- 保留内存配置和两个 Forge 兼容参数，使用离线 `--session 0`、`--tweakClass cpw.mods.fml.common.launcher.FMLTweaker`，游戏工作目录仍是 `.minecraft`。不会覆盖原版依赖、游戏 jar 或模组。
- 缺少声明的依赖 jar、替换 jar 或 ARM natives 目录时，停止启动并报告路径，不静默遗漏依赖。

此分支面向脚本对应的旧版 **LWJGL 2 / Forge 整合包**，不是 LWJGL 3 的通用启动方式。需自行准备脚本配套的 `mac-dlls/dist/` 产物（启动器不会自动下载或构建），并在设置中选择 **arm64 / aarch64 Java 8**；Intel Java 不能加载这些 ARM 动态库。不要额外添加 `-XstartOnFirstThread`。


## macOS Java 自动寻找

启动器恢复配置后自动扫描；在 **设置 → 本机 Java** 查看 Java 可执行文件的完整路径、版本、厂商及指令集架构（如 `arm64`、`x86_64`、`x86`）。点击 **选用** 将路径填入现有设置并自动保存；扫描和重新扫描本身不会覆盖当前选择。

扫描来源：

- macOS `/usr/libexec/java_home -V`、系统与用户的 `JavaVirtualMachines` 目录、旧版 Apple Java 和 Java 插件目录。
- `JAVA_HOME`、`PATH`，以及 `/opt/homebrew` 和 `/usr/local` 下 Homebrew 的 `opt` / `Cellar` JDK。
- SDKMAN、asdf、mise/rtx、jabba、jenv、Gradle 的常见 Java 目录；支持 SDKMAN、asdf、mise、jabba 的自定义根目录环境变量。
- Minecraft、PrismLauncher、PolyMC 的常见运行时目录，以及 `/Applications`、`~/Applications` 下常见应用内置的 JBR/JRE。

每个候选实际运行 `-XshowSettings:properties -version`，从 JVM 输出读取信息；仅在旧 JVM 明确不支持该选项时回退 `-version`，无法确认的架构显示为“未知”，不会拿本机 CPU 架构代替 JVM 架构。结果按真实可执行文件路径去重，忽略 macOS `/usr/bin/java` 安装占位程序。

扫描不全盘递归：常见目录有深度限制，最多深入 4096 个目录、接受 128 个不同候选；目录软链接只检查已知 Java home 结构，不递归。每次探测超时为 4 秒，输出上限为 128 KiB；不可运行、超时、权限不足或达到扫描上限等情况显示在 **扫描提示** 中，不影响其他有效结果。自定义位置、无法读取的目录或不兼容当前 macOS 的 Java 可能不会列出，仍可手动填写路径并检测。

## 项目结构

```
src/                     前端（TypeScript + Vite，无框架）
  main.ts                入口：初始化各页面 → 恢复配置 → 识别 .minecraft
  config.ts              设置表单 ↔ 配置模型（防抖持久化、表单回填）
  assets/logo.png        应用图标源图（`npx tauri icon src/assets/logo.png` 重新生成 src-tauri/icons）
  api/types.ts           与 Rust 结构体一一对应的 payload 类型
  api/backend.ts         唯一接触 invoke / listen / 原生对话框的模块
  ui/dom.ts              DOM id → 元素引用（唯一出现 id 字符串的地方）
  ui/nav.ts              页面切换
  ui/status.ts           状态胶囊 / Java 检测行 / 识别提示
  ui/profile.ts          主页玩家卡
  ui/memory.ts           内存滑块与数字输入联动
  pages/home.ts          主页：启动按钮、game-log / game-exit 事件
  pages/settings.ts      设置页：表单绑定、Java 浏览/检测、自动寻找与选用
  pages/logs.ts          日志页：采集开关、实时控制台、导出
  styles.css             样式入口（@import 下按页面拆分的片段）
  styles/*.css           base / sidebar / home / settings / logs / buttons

src-tauri/src/           Rust 后端，按职责拆分
  lib.rs                 模块声明 + Tauri 启动（命令注册、Windows WebView2 修正）
  config.rs              配置读写（便携目录下的 JSON）
  game.rs                .minecraft 目录与版本识别
  java.rs                Java 版本/厂商/架构探测、超时进程管理与命令接口
  java/discovery.rs      macOS Java 多来源有界扫描、路径去重
  version.rs             版本 json 解析（主类 / tweakClass）
  classpath.rs           classpath 与 natives 解析
  launch.rs              命令行构造 + 进程托管
  log_export.rs          导出日志写入
  platform.rs            便携目录定位 / no_window / WebView2 autofill
  fixtures.rs            仅测试编译的共享夹具
  real_client_tests.rs   需真实客户端 + javaw 的 --ignored 测试
```

## 开发

```bash
npm install
npm run tauri dev    # 开发（注意：dev 时启动器 cwd 为 src-tauri，需在其下放 .minecraft 才能识别）
npm run tauri build  # 通用打包；macOS 便携应用使用下面的专用命令
```

Rust 侧测试：`cd src-tauri && cargo test`（含针对本机真实客户端的 `--ignored` 集成测试）。

## macOS 打包与使用

```bash
make mac-release
```

该 `.PHONY` 目标复用 `npm run build:macos`，构建带 Finder 图标、双击无需 Terminal 的完整 `.app`。

生成的应用位于 `src-tauri/target/release/bundle/macos/Nova Launcher.app`。发布、复制和双击打开的对象必须是完整的 `.app`，它包含 Finder 图标和原生应用启动信息，正常打开不会创建 Terminal 窗口。

不要将 `src-tauri/target/release/nova-launcher` 或 `.app/Contents/MacOS/nova-launcher` 当作 macOS 发布应用：它们是裸 Mach-O 可执行文件，Finder 不会使用 `.app` 的图标，双击可能通过 Terminal 运行。

便携目录布局：

```text
游戏目录/
  Nova Launcher.app
  .minecraft/
  mac-dlls/                 # Apple Silicon 的 LWJGL 2 整合包必需
    dist/
      lwjgl.jar
      lwjgl_util.jar
      jinput-2.0.5.jar
      jutils-1.0.0.jar
      natives/
        liblwjgl.dylib
        openal.dylib
        libjinput-osx.dylib
  nova_launcher_config.json  # 设置修改后自动保存
```

`.minecraft` 必须放在 `.app` **外部同级目录**，而不是 `Contents/MacOS` 内。macOS 打包应用及发布版裸程序按自身位置定位数据，不会主动切换进程工作目录；未打包的 `tauri dev` 保持上面的开发目录约定。

