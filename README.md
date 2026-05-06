# 🚀 AI Clipboard Manager (AI 剪贴板助手)

一个基于 **Tauri 2.0**、**Rust** 和 **React** 构建的现代化、高性能 AI 剪贴板管理工具。

[![GitHub release](https://img.shields.io/github/v/release/loutaiyue/AI-Clipboard-Manager?style=flat-square)](https://github.com/loutaiyue/AI-Clipboard-Manager/releases)
[![License](https://img.shields.io/github/license/loutaiyue/AI-Clipboard-Manager?style=flat-square)](https://github.com/loutaiyue/AI-Clipboard-Manager/blob/main/LICENSE)

## 🌟 项目亮点

传统的剪贴板工具只负责“存”，而我们负责“懂”。通过集成 DeepSeek 等大语言模型，这个小工具可以自动识别你复制的内容并给出智能建议。

- **⚡ 极致性能**：采用 Rust 编写后端，内存占用极低，运行丝滑。
- **🧠 AI 智能驱动**：自动总结长文、翻译外语、解释代码逻辑。
- **💾 永久记忆**：本地持久化存储记录，即使重启电脑，重要的复制内容也不会丢失。
- **🎨 现代 UI**：符合 Windows 11 设计语言的毛玻璃（Mica）风格界面。
- **🔒 隐私第一**：所有数据存储在本地，API 直接请求厂商，不经过任何中转服务器。

## 📸 软件预览

> [此处建议上传你在 Cursor 中运行软件的截图，替换下方占位符]
![Software Screenshot](https://raw.githubusercontent.com/loutaiyue/AI-Clipboard-Manager/main/assets/screenshot.png)

## 🛠️ 技术栈

- **Frontend**: React + Vite + Tailwind CSS
- **Backend**: Rust + Tauri 2.0
- **AI Engine**: DeepSeek-V4-Flash / DeepSeek-R1 (支持自定义 API)
- **Database**: Local JSON File Persistence

## 🚀 快速开始

### 下载安装
目前支持 Windows 10/11 系统。
1. 前往 [Releases](https://github.com/loutaiyue/AI-Clipboard-Manager/releases) 页面。
2. 下载最新的 `AI-Clipboard-Manager.exe` (绿色版)。
3. 双击即可运行，无需安装。

### 开发者模式 (本地构建)
如果你想自己修改代码或进行二次开发：

```bash
# 克隆仓库
git clone [https://github.com/loutaiyue/AI-Clipboard-Manager.git](https://github.com/loutaiyue/AI-Clipboard-Manager.git)

# 进入目录
cd AI-Clipboard-Manager

# 安装前端依赖
npm install

# 运行开发环境
npm run tauri dev

# 打包发布版
npm run tauri build
