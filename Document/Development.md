# 开发与构建

本文面向 Nova Launcher 的开发与维护者，介绍本地开发、发布构建和启动实现。项目介绍与玩家使用说明见[项目首页](../README.md)。

## 技术栈与目录

项目使用 Tauri 2，前端为 TypeScript + Vite（无 UI 框架），后端为 Rust。

```text
src/
  main.ts                   页面初始化、配置恢复与游戏识别
  config.ts                 设置表单与配置同步、防抖保存
  api/                      前后端数据类型及 invoke / listen / 原生对话框封装
  ui/                       DOM 引用、导航、状态、玩家卡与内存控件
  pages/                    主页、设置、日志页面逻辑
  styles.css                样式入口
  styles/                   按页面与组件拆分的样式
  assets/logo.png            应用图标源图
src-tauri/
  src/
    lib.rs                  Tauri 初始化与命令注册
    config.rs               便携配置读写
    game.rs                 游戏目录识别与版本选择
    java.rs                 Java 信息探测与命令接口
    java/discovery.rs       有界扫描、macOS 来源与路径去重
    java/discovery/windows.rs  Windows 注册表与安装目录来源
    version.rs              主类与 tweakClass 解析
    classpath.rs            Java 依赖与 native 路径解析
    launch.rs               启动命令构造与游戏进程管理
    log_export.rs           日志导出
    platform.rs             平台路径、进程窗口与 WebView2 适配
    fixtures.rs             测试夹具
    real_client_tests.rs    依赖真实客户端的忽略测试
  tauri.conf.json           应用与打包配置
Document/                   开发文档与原始需求记录
```

## 本地开发

准备 Node.js、npm、Rust 工具链及当前平台的 Tauri 构建依赖。Windows 使用 Rust MSVC 工具链，并安装 Visual Studio C++ 构建工具。

在仓库根目录运行：

```bash
npm install
npm run tauri dev
```

开发模式以 `src-tauri/` 为工作目录，测试客户端应放在 `src-tauri/.minecraft/`。Apple Silicon 开发环境还需在 `src-tauri/mac-dlls/dist/` 放置配套依赖，布局见下文。

常用检查命令：

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

Rust 测试中的 `--ignored` 用例依赖本机真实客户端与 `javaw`，不属于默认测试集；运行前应查看 `src-tauri/src/real_client_tests.rs` 中的环境要求。

## 发布构建

在目标平台、使用目标架构的工具链执行对应命令。以下平台命令均会先构建前端，再构建 release 应用。

| 平台 | 命令 | 产物 |
| --- | --- | --- |
| Windows | `npm run build:windows` 或 `make windows-release` | `src-tauri/target/release/nova-launcher.exe` 与 `src-tauri/target/release/bundle/nsis/` 下的 NSIS 安装包 |
| macOS | `npm run build:macos` 或 `make mac-release` | `src-tauri/target/release/bundle/macos/Nova Launcher.app` |

`npm run tauri build` 使用通用打包配置。Apple Silicon 发布应使用 arm64 工具链；构建脚本不会主动切换目标架构。

macOS 分发完整的 `.app`，保留 Finder 图标及原生应用启动信息，不以裸 Mach-O 文件或 `.app/Contents/MacOS/` 内的程序替代。正常双击 `.app` 不会创建 Terminal 窗口。

Windows NSIS 安装包使用 Tauri 默认的 WebView2 引导安装方式；直接分发 exe 时，运行环境需已安装 WebView2 Runtime。

应用图标由 `src/assets/logo.png` 生成，更新命令为：

```bash
npx tauri icon src/assets/logo.png
```

## 数据目录约定

游戏资源与配置使用便携目录，不写入系统应用配置目录：

| 运行方式 | 便携目录 |
| --- | --- |
| macOS `.app` | `.app` 所在目录 |
| macOS 未打包 release 程序 | 可执行文件所在目录 |
| Windows、其他平台及未打包开发模式 | 进程工作目录 |

启动器从便携目录读取 `.minecraft/`，设置保存为同目录下的 `nova_launcher_config.json`。Windows 分发或创建快捷方式时，需将工作目录设为游戏目录；macOS 打包应用不依赖 Finder 或 Terminal 的工作目录。

Apple Silicon 发布布局：

```text
游戏目录/
  Nova Launcher.app
  .minecraft/
  mac-dlls/
    dist/
      lwjgl.jar
      lwjgl_util.jar
      jinput-2.0.5.jar
      jutils-1.0.0.jar
      natives/
        liblwjgl.dylib
        openal.dylib
        libjinput-osx.dylib
  nova_launcher_config.json
```

`.minecraft/` 与 `mac-dlls/` 均位于 `.app` 外部。`mac-dlls/dist/` 需要另行准备，启动器不会自动下载或构建这些文件。

## 游戏启动实现

### 通用流程

1. 识别便携目录下的 `.minecraft/`，从 `versions/` 中选择版本 jar 修改时间最新的版本。
2. 读取玩家名、Java 路径及内存配置；玩家名中的空格转换为 `_`，使用离线会话启动。
3. 按版本 JSON 选择已存在的非 native 依赖；没有可读取的 JSON 时递归收集 `libraries/**/*.jar`，最后追加版本 jar。
4. 从版本 JSON 解析主类与 `tweakClass`，缺少 JSON 时按版本名推断；native 目录使用 `.minecraft/natives` 或版本目录内匹配的目录。
5. 以 `.minecraft/` 为游戏工作目录启动 Java，管理进程退出事件，并按日志采集设置转发输出。

当前没有正版登录或版本隔离。版本选择是自动行为，不限定版本目录名为 `1.6.4`。

### Apple Silicon 适配

Apple Silicon 原生启动器及该硬件上经 Rosetta 运行的 Intel 启动器均使用专用启动分支，按配套 `mac-dlls/launch-modpack-arm64.py` 的 LWJGL 2 方案构造命令。该方案面向旧版 LWJGL 2 / Forge 整合包，不是 LWJGL 3 的通用启动方式。

- 必须读取版本 JSON 中的 `mainClass`、`id` 与 `libraries`。
- 依赖按 JSON 顺序排列，不应用旧版 OS rules；移除全部 `org.lwjgl.lwjgl` 依赖，不将 native jar 放入 classpath。
- 将 JInput、JUtils 替换为 `mac-dlls/dist/` 下的对应 jar，再追加该目录下的 `lwjgl.jar`、`lwjgl_util.jar` 与版本 jar。
- 将 `java.library.path`、`org.lwjgl.librarypath` 和 `net.java.games.input.librarypath` 指向 `mac-dlls/dist/natives/`，不回退到旧 native 目录。
- 保留内存设置与两个 Forge 兼容参数，使用 `--session 0` 和 `--tweakClass cpw.mods.fml.common.launcher.FMLTweaker`。
- 缺少声明依赖、替换 jar 或 ARM natives 目录时，返回包含路径的错误，不静默遗漏依赖。

运行时使用 **arm64 / aarch64 Java 8**，Intel Java 无法加载配套 ARM 动态库。此方案不添加 `-XstartOnFirstThread`，也不覆盖原有游戏 jar、依赖或模组。

## Java 发现与探测

Windows 与 macOS 在恢复配置后自动扫描本机 Java。结果包含可执行文件路径、版本、厂商和 JVM 架构；只有点击“选用”才会更新并保存 Java 路径，扫描本身不覆盖当前选择。其他平台可手动指定路径并检测。

### 搜索来源

| 平台 | 来源 |
| --- | --- |
| Windows | 已保存路径；`JAVA_HOME`、`JRE_HOME`、`JDK_HOME`、`PATH`；HKLM 的 32 / 64 位视图及 HKCU 中的常见 Java 安装键；系统与用户安装目录；Scoop、Chocolatey；常见 Java 版本管理器、Gradle、JetBrains / Android Studio 与 Toolbox；Minecraft、PrismLauncher、PolyMC 运行时 |
| macOS | `/usr/libexec/java_home -V`；系统与用户 `JavaVirtualMachines`、旧版 Apple Java 与插件目录；`JAVA_HOME`、`PATH`；Homebrew；常见 Java 版本管理器与 Gradle；Minecraft、PrismLauncher、PolyMC 运行时；系统与用户 Applications 下的常见应用内置 JBR/JRE |

Windows 优先探测 `java.exe`，同目录的 `javaw.exe` 不重复展示；只有 GUI 可执行文件时保留该候选。Oracle `javapath` 与 JDK 内置 JRE 按 JVM 报告的 `java.home` 归并。注册表只读，最多扫描 1024 个键、向下 4 层，支持环境变量形式的安装路径。

### 探测边界

- 候选运行 `-XshowSettings:properties -version`，仅在旧 JVM 明确不支持该选项时回退到 `-version`。
- 架构取自 JVM 输出，无法确认时显示“未知”，不以主机 CPU 架构代替。结果按真实可执行文件路径去重，并忽略 macOS `/usr/bin/java` 占位程序。
- 扫描仅覆盖有深度限制的常见目录，最多进入 4096 个目录、接受 128 个不同候选；软链接及 Windows junction / reparse point 仅检查已知 Java home 结构，不递归深入。
- 单次探测超时为 4 秒，输出上限为 128 KiB。超时、权限不足、不可运行或扫描达到上限等情况列入“扫描提示”，不影响有效结果。

自定义安装位置或无法扫描的 Java 仍可在设置中手动指定并检测。
