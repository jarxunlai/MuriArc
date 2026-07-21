# MuriArc Importer

`muriarc-importer` 是不依赖数据库和 transport 的同步领域组件，负责把 CSV/XLSX 转换为可审阅的导入预览，并生成经过过滤的动物 CSV/XLSX 导出。

## 边界

- 本 crate **不连接 SQLite/PostgreSQL**，也不调用 Tauri、Axum、REST 或 MCP。
- 调用方先从 Store 构造 `AnimalDirectory` 与 `MeasurementCatalog`，再将其作为只读目录传入预览函数。
- 预览只返回已解析行和稳定 issue code；确认后的事务写入、权限、审计及报告属于 application/Job 层。
- 重复提交由 Job 的 idempotency key 管理；Importer 不保存任务状态，也不把“相同文件”猜测为同一提交。
- 解析、预览和导出均提供 cooperative cancellation hook。取消只停止当前操作；调用方负责丢弃尚未确认的缓冲区或临时文件。

## 安全流程

```text
parse → validate/map → preview → user confirmation → transactional write → report
```

Measurement 导入必须显式提供动物显示编号、指标 key、值类型、值、单位和测量时间。显示编号先解析为动物 UUID；未知或歧义编号、非法/缺失单位、非有限数值以及同一动物/指标/时间的重复行都会阻断确认。
