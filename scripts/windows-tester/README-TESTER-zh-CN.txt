MuriArc 1.0.0 Windows Tester
============================

警告：unsigned / synthetic data / not for production

这是朋友测试专用的未签名调试包，不是正式 v1.0.0 Release，也不构成 RC PASS。
包内只包含由本包 MuriArc Desktop 二进制自身生成并验证的 standard-v1 合成数据，
不包含真实科研数据、真实账号密码或 AI API Key。

运行环境：Windows 10/11 x64，并已安装 Microsoft Edge WebView2 Runtime。
此包没有安装程序，也不会替换正式版 MuriArc；不同源码 commit 使用独立应用标识和数据目录。

使用方法：

1. 完整解压 ZIP，不要直接在压缩软件内运行。
2. 使用 Get-FileHash -Algorithm SHA256 核对 ZIP，并与同一 prerelease 的 .sha256 文件比较。
3. 双击 Start-MuriArc-Tester.cmd。Windows SmartScreen 可能提示未知发布者，因为此包未签名。
4. 启动器先逐文件验证 CHECKSUMS.sha256、Tester manifest、Desktop EXE 和 synthetic baseline；
   任一项不一致都会停止，不会保存或启动被篡改的包。
5. 首次启动时，基线会复制到当前用户的 LocalAppData；之后的人工测试修改会保留，
   不会回写 ZIP 解压目录中的 standard-v1 基线。
6. 源码 commit、数据 digest、独立应用标识与安全分类见 TESTER-MANIFEST.json。

反馈问题时请提供：Tester tag、source commit、复现步骤和错误截图。不要发送任何真实动物数据、
AI API Key、密码、Session、Token 或其他密钥。

请勿把此包或其中产生的测试修改用于生产、正式 RC 或真实动物数据。
