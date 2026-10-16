import type { TokenUsage } from "./types";

const weekdayFormat = new Intl.DateTimeFormat("ko-KR", { weekday: "short" });

const pad = (n: number) => String(n).padStart(2, "0");

// 모든 시각을 같은 양식으로 보여준다: "11·13(금) 22:56"
function formatDateTime(ms: number): string {
  const date = new Date(ms);
  const day = `${pad(date.getMonth() + 1)}·${pad(date.getDate())}(${weekdayFormat.format(date)})`;
  return `${day} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

// 다음 재설정 시각. "11·13(금) 22:56"
export function formatResetTime(ms: number): string {
  return formatDateTime(ms);
}

// 지난 시각. "10·14(수) 22:57"
export function formatPastTime(ms: number): string {
  return formatDateTime(ms);
}

const dayFormat = new Intl.DateTimeFormat("ko-KR", {
  month: "numeric",
  day: "numeric",
  weekday: "short",
});

// "10. 13. (화)"
export function formatDay(ms: number): string {
  return dayFormat.format(new Date(ms));
}

export function totalTokens(t: TokenUsage): number {
  return t.input + t.output + t.cacheWrite + t.cacheRead;
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}
