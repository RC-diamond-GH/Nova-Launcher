# Nova Launcher

Minecraft 启动器 MVP（Tauri 2 + Rust + TypeScript），淡蓝色主题。

## 界面

左侧窄图标栏切换三个页面：

- **主页**：玩家名 + 启动按钮
- **设置**：玩家名、Java 可执行文件（浏览/`-version` 检测）、内存分配
- **日志**：启用日志采集开关（下次启动生效）、导出日志按钮、实时日志

## 启动逻辑

- `.minecraft` 目录不做设置，自动识别**启动器当前目录**下的 `.minecraft`
- 版本自动选择：`versions/` 下 jar 修改时间最新的版本
- classpath：递归收集 `libraries/**/*.jar` + 版本 jar；natives 取 `.minecraft/natives`（或版本目录内 `*natives*`）
- 主类 / tweakClass 自动从版本 json 解析（如 Forge 的 `net.minecraft.launchwrapper.Launch` + `FMLTweaker`），无 json 时按版本名推断
- 离线用户名（空格转 `_`），配置持久化到程序运行目录下的 `nova_launcher_config.json`（便携，不写系统应用配置目录）

第一版不做：自动寻找 Java、正版登录、版本隔离。

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
  pages/settings.ts      设置页：表单绑定、Java 浏览与检测
  pages/logs.ts          日志页：采集开关、实时控制台、导出
  styles.css             样式入口（@import 下按页面拆分的片段）
  styles/*.css           base / sidebar / home / settings / logs / buttons

src-tauri/src/           Rust 后端，按职责拆分
  lib.rs                 模块声明 + Tauri 启动（命令注册、Windows WebView2 修正）
  config.rs              配置读写（便携：程序运行目录下的 JSON）
  game.rs                .minecraft 目录与版本识别
  java.rs                java -version 探测
  version.rs             版本 json 解析（主类 / tweakClass）
  classpath.rs           classpath 与 natives 解析
  launch.rs              命令行构造 + 进程托管
  log_export.rs          导出日志写入
  platform.rs            no_window / WebView2 autofill
  fixtures.rs            仅测试编译的共享夹具
  real_client_tests.rs   需真实客户端 + javaw 的 --ignored 测试
```

## 开发

```bash
npm install
npm run tauri dev    # 开发（注意：dev 时启动器 cwd 为 src-tauri，需在其下放 .minecraft 才能识别）
npm run tauri build  # 打包
```

Rust 侧测试：`cd src-tauri && cargo test`（含针对本机真实客户端的 `--ignored` 集成测试）。
