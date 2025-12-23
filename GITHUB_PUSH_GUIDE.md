# GitHub 推送指南

## 当前状态

✅ Git 仓库已初始化
✅ 所有文件已提交
✅ GitHub Actions 工作流已配置
✅ Remote 已设置为 https://github.com/NeolnaX/UA-Forge.git

## 推送到 GitHub

### 方法 1: 使用 GitHub CLI (推荐)

```bash
# 安装 GitHub CLI (如果未安装)
# Ubuntu/Debian:
sudo apt install gh

# 登录 GitHub
gh auth login

# 推送代码
git push -u origin main
```

### 方法 2: 使用 Personal Access Token

1. 访问 https://github.com/settings/tokens
2. 点击 "Generate new token (classic)"
3. 选择权限: `repo` (完整仓库访问)
4. 生成并复制 token

```bash
# 推送时会提示输入用户名和密码
# 用户名: 你的 GitHub 用户名
# 密码: 粘贴刚才生成的 token
git push -u origin main
```

### 方法 3: 使用 SSH

```bash
# 修改 remote URL 为 SSH
git remote set-url origin git@github.com:NeolnaX/UA-Forge.git

# 推送
git push -u origin main
```

## GitHub Actions 自动编译

推送成功后，GitHub Actions 会自动开始编译：

### 触发条件
- ✅ 推送到 main 分支
- ✅ 创建 Pull Request
- ✅ 创建版本标签 (v*)
- ✅ 手动触发

### 编译架构
1. **MIPS (mipsel_24kc)** - MT7621 路由器
2. **ARM64 (aarch64_cortex-a53)** - 树莓派等
3. **x86_64** - x86 软路由

### 查看编译状态
访问: https://github.com/NeolnaX/UA-Forge/actions

## 创建 Release

### 自动创建 Release

```bash
# 创建版本标签
git tag -a v0.1.1 -m "Release v0.1.1"

# 推送标签
git push origin v0.1.1
```

GitHub Actions 会自动：
1. 编译所有架构的 IPK 包
2. 创建 GitHub Release
3. 上传所有 IPK 文件到 Release

### 手动创建 Release

1. 访问 https://github.com/NeolnaX/UA-Forge/releases
2. 点击 "Draft a new release"
3. 选择标签或创建新标签
4. 填写 Release 说明
5. 发布

## 编译产物

编译成功后，可以在以下位置找到 IPK 包：

- **Artifacts**: https://github.com/NeolnaX/UA-Forge/actions (每次构建)
- **Releases**: https://github.com/NeolnaX/UA-Forge/releases (版本发布)

### IPK 文件命名
- `uaforge_0.1.1-1_mipsel_24kc.ipk` - MIPS 架构
- `uaforge_0.1.1-1_aarch64_cortex-a53.ipk` - ARM64 架构
- `uaforge_0.1.1-1_x86_64.ipk` - x86_64 架构

## 故障排查

### 编译失败

1. 检查 Actions 日志: https://github.com/NeolnaX/UA-Forge/actions
2. 查看具体错误信息
3. 常见问题:
   - SDK 下载失败: 检查网络连接
   - 依赖缺失: 检查 Makefile 中的依赖声明
   - 编译错误: 检查 Rust 代码

### 推送失败

```bash
# 检查 remote 配置
git remote -v

# 检查认证状态
gh auth status  # 如果使用 GitHub CLI

# 强制推送 (谨慎使用)
git push -f origin main
```

## 下一步

1. ✅ 推送代码到 GitHub
2. ✅ 等待 Actions 编译完成
3. ✅ 创建版本标签触发 Release
4. ✅ 下载 IPK 包测试安装

## 相关链接

- GitHub 仓库: https://github.com/NeolnaX/UA-Forge
- Actions 页面: https://github.com/NeolnaX/UA-Forge/actions
- Releases 页面: https://github.com/NeolnaX/UA-Forge/releases
- Issues 页面: https://github.com/NeolnaX/UA-Forge/issues
