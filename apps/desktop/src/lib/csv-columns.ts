import type { CsvColumn } from './types'

export const csvLabels: Record<CsvColumn,string> = {
  code:'编号', title:'标题', year:'年份', actors:'演员', score:'评分', parts:'分片数', favorite:'收藏', tags:'标签', paths:'文件路径',
}
export const standardColumns: CsvColumn[] = ['code','title','year','actors','score','parts','favorite']
