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

const monthDayFormat = new Intl.DateTimeFormat("ko-KR", { month: "long", day: "numeric" });

// 지난 시각. 오늘이면 "오늘 오후 2:30", 다른 날이면 "10월 13일 오후 7:30"
export function formatPastTime(ms: number, now = new Date()): string {
  const date = new Date(ms);
  const time = timeFormat.format(date);
  if (date.toDateString() === now.toDateString()) {
    return `오늘 ${time}`;
  }
  return `${monthDayFormat.format(date)} ${time}`;
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
