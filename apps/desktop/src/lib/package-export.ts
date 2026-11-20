export type PackageFileKind='nfo'|'poster'|'fanart'|'avatar'|'sidecar'|'video'|'subtitle'
export interface PackageFile {kind:PackageFileKind; path:string; bytes:number; sha256:string; actor?:string; source?:string}
export interface PackageManifest {version:number; movies:{code:string; files:PackageFile[]}[]}
export interface PackageExportResult {destination:string; movies:number; files:number; bytes:number; examples:string[]; warnings:string[]; warningCount:number}
export interface PackageExportPreview extends PackageExportResult {token:string}
export interface PackageExportApi {
  packagePreview(codes:string[],kinds:PackageFileKind[],directory:string,requestId?:string):Promise<PackageExportPreview>
  packageExport(token:string,requestId?:string):Promise<PackageExportResult>
}
export function nativePackageExport(invoke:<T>(command:string,args:Record<string,unknown>)=>Promise<T>):PackageExportApi {
  return {
    packagePreview:(codes,kinds,directory,requestId)=>invoke('package_preview',{codes,kinds,directory,requestId}),
    packageExport:(token,requestId)=>invoke('package_export',{token,requestId}),
  }
}
