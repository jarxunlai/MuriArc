import type { AiWriteDraft } from './models'

export const AI_IMPORT_PREVIEW_ROW_LIMIT = 20

export function isBulkMeasurementDraft(draft: AiWriteDraft): boolean {
  return draft.kind === 'bulk_import' || draft.kind === 'bulk_measurement'
}

/**
 * Returns the reason an import cannot be approved, or undefined when the
 * persisted server preview is internally coherent for the selected project.
 *
 * The server remains authoritative and revalidates immediately before apply;
 * this is a fail-closed UI/composable guard against absent, stale, or malformed
 * approval projections.
 */
export function aiImportApprovalBlockReason(
  draft: AiWriteDraft,
  selectedProjectId?: string,
): string | undefined {
  if (!isBulkMeasurementDraft(draft)) return undefined

  const preview = draft.importPreview
  if (!preview) return '缺少正式导入预览，无法批准批量测量导入'
  if (preview.importKind !== 'measurement') return '该草稿不是受支持的批量测量导入'
  if (!preview.canConfirm) return '导入预览尚未通过服务端校验，无法批准'
  if (!selectedProjectId
    || draft.projectId !== selectedProjectId
    || preview.projectId !== selectedProjectId) {
    return '导入预览与当前科研项目不一致'
  }
  if (!preview.experimentId?.trim()) return '导入预览缺少目标实验'
  if (!preview.fileName?.trim() || !preview.sheetName?.trim()) {
    return '导入预览缺少文件或工作表信息'
  }
  if (preview.issues.some((issue) => issue.severity === 'error')) {
    return '导入预览仍包含错误，无法批准'
  }
  if (!Number.isInteger(preview.totalRows)
    || !Number.isInteger(preview.acceptedRows)
    || !Number.isInteger(preview.issueCount)
    || preview.totalRows < 0
    || preview.acceptedRows < 0
    || preview.acceptedRows > preview.totalRows
    || preview.issueCount < preview.issues.length
    || preview.previewRows.length > AI_IMPORT_PREVIEW_ROW_LIMIT
    || preview.previewRows.length > preview.acceptedRows
    || preview.previewRowsTruncated !== (preview.acceptedRows > preview.previewRows.length)
    || preview.issuesTruncated !== (preview.issueCount > preview.issues.length)) {
    return '导入预览统计不完整或不一致'
  }
  return undefined
}
