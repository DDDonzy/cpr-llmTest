import type { Account } from './api'

export interface DraftTask {
  key: string
  index: number
  accountId: string
  accountName: string
  model: string
  effort: string
}
// Tuple encoding avoids collisions from model names containing separators.
export function selectionKey(accountId: string, model: string, effort: string): string {
  return JSON.stringify([accountId, model, effort])
}
export function buildDraftTasks(accounts: Account[], ids: string[], models: string[], efforts: string[]): DraftTask[] {
  const names = new Map(accounts.map(a => [a.account_id, a.email || a.name]))
  const rows: DraftTask[] = []
  for (const accountId of ids) for (const model of models) for (const effort of efforts) {
    if (rows.length >= 500) return rows
    rows.push({ key: selectionKey(accountId, model, effort), index: rows.length, accountId, accountName: names.get(accountId) || accountId, model, effort })
  }
  return rows
}
export function selectedIndexes(rows: DraftTask[], excluded: ReadonlySet<string>): number[] {
  return rows.filter(row => !excluded.has(row.key)).map(row => row.index)
}
