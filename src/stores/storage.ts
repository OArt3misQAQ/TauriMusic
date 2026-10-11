/** localStorage 统一管理:键名集中、防散落 */
const PREFIX = "tm-";

export const StorageKeys = {
  volume: `${PREFIX}vol`,
  welcome: `${PREFIX}welcome`,
  theme: `${PREFIX}theme`,
  skin: `${PREFIX}skin`,
  mode: `${PREFIX}mode`,
  /** 听歌统计总开关("0"=关闭):只管偏好,数据本体在 Rust 端 stats.json */
  stats: `${PREFIX}stats`,
} as const;

export function storageGet(key: string): string | null {
  return localStorage.getItem(key);
}

export function storageGetString(key: string, fallback: string): string {
  return localStorage.getItem(key) ?? fallback;
}

export function storageSet(key: string, value: string): void {
  localStorage.setItem(key, value);
}

export function storageRemove(key: string): void {
  localStorage.removeItem(key);
}
