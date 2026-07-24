# Run Cmd

一个用 Tauri 2 + Vue 3 做的桌面命令启动器，面向前端项目，尤其适合 Vue 项目。

## 功能

- 选择或拖拽包含 `package.json` 的项目目录
- 自动读取并展示 `package.json` 里的 `scripts`
- 左侧项目列表，右侧选中项目的命令列表
- 点击运行或停止某个命令
- 项目和命令都支持拖拽排序
- 排序和已添加项目会保存在本地，下次打开自动恢复

## 开发运行

先安装依赖：

```bash
npm install
```

安装 Rust 工具链后运行：

```bash
npm run tauri:dev
```

## 打包 Windows exe

在 Windows 机器上安装 Node.js 和 Rust 后执行：

```bash
npm install
npm run tauri:build
```

构建产物会在：

```text
src-tauri/target/release/bundle/nsis/
```

Tauri 默认会生成 NSIS 安装包 `.exe`。
