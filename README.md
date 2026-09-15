# Nova Launcher

Minecraft 启动器 MVP（Tauri 2 + Rust + TypeScript），淡蓝色主题。

## 界面

左侧窄图标栏切换三个页面：

- **主页**：玩家名 + 启动按钮，右侧预留拓展区
- **设置**：玩家名、Java 可执行文件（浏览/`-version` 检测）、内存分配
- **日志**：启用日志采集开关（下次启动生效）、导出日志按钮、实时日志

## 启动逻辑

- `.minecraft` 目录不做设置，自动识别**启动器当前目录**下的 `.minecraft`
- 版本自动选择：`versions/` 下 jar 修改时间最新的版本
- classpath：递归收集 `libraries/**/*.jar` + 版本 jar；natives 取 `.minecraft/natives`（或版本目录内 `*natives*`）
- 主类 / tweakClass 自动从版本 json 解析（如 Forge 的 `net.minecraft.launchwrapper.Launch` + `FMLTweaker`），无 json 时按版本名推断
- 离线用户名（空格转 `_`），配置持久化到程序运行目录下的 `nova_launcher_config.json`（便携，不写系统应用配置目录）

第一版不做：自动寻找 Java、正版登录、版本隔离。

## 开发

```bash
npm install
npm run tauri dev    # 开发（注意：dev 时启动器 cwd 为 src-tauri，需在其下放 .minecraft 才能识别）
npm run tauri build  # 打包
```

Rust 侧测试：`cd src-tauri && cargo test`（含针对本机真实客户端的 `--ignored` 集成测试）。
