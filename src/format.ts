import type { TokenUsage } from "./types";

const timeFormat = new Intl.DateTimeFormat("ko-KR", {
  hour: "numeric",
  minute: "2-digit",
});
const weekdayFormat = new Intl.DateTimeFormat("ko-KR", { weekday: "long" });

// 오늘이면 "오후 7:30", 다른 날이면 "월요일 오전 1:00"
export function formatResetTime(ms: number, now = new Date()): string {
  const date = new Date(ms);
  const time = timeFormat.format(date);
  if (date.toDateString() === now.toDateString()) {
    return time;
  }
  return `${weekdayFormat.format(date)} ${time}`;
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
