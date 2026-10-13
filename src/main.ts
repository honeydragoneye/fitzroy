import { invoke } from "@tauri-apps/api/core";
import { formatResetTime, formatTokens, totalTokens } from "./format";
import type { ProviderReport } from "./types";

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function limitRow(
  label: string,
  percent: number | null,
  resetsAt: number | null,
): HTMLElement | null {
  if (percent === null) return null;

  const row = el("div", "limit");
  const head = el("div", "limit-head");
  head.append(el("span", "limit-label", label));
  if (resetsAt !== null) {
    head.append(el("span", "muted", `${formatResetTime(resetsAt)}에 재설정`));
  }

  const bar = el("div", "bar");
  const fill = el("div", "bar-fill");
  fill.style.width = `${Math.min(100, Math.max(0, percent))}%`;
  bar.append(fill);

  row.append(head, bar, el("div", "muted", `${Math.round(percent)}% 사용됨`));
  return row;
}

function statRow(label: string, value: string): HTMLElement {
  const row = el("div", "stat");
  row.append(el("span", "muted", label), el("span", undefined, value));
  return row;
}

function hourlyChart(hours: number[]): HTMLElement {
  const chart = el("div", "chart");
  const max = Math.max(...hours, 1);
  for (const value of hours) {
    const bar = el("div", "chart-bar");
    bar.style.height = `${Math.max(value > 0 ? 6 : 0, (value / max) * 100)}%`;
    chart.append(bar);
  }
  chart.title = "최근 24시간 시간별 토큰";
  return chart;
}

function card(report: ProviderReport): HTMLElement {
  const node = el("section", "card");
  node.append(el("h2", "card-title", report.name));

  const session = limitRow("현재 세션", report.sessionPercent, report.sessionResetsAt);
  const weekly = limitRow("이번 주", report.weeklyPercent, report.weeklyResetsAt);
  if (session) node.append(session);
  if (weekly) node.append(weekly);

  if (report.tokensToday) {
    const t = report.tokensToday;
    node.append(
      el("div", "tokens", `오늘 ${formatTokens(totalTokens(t))} 토큰`),
      el(
        "div",
        "muted",
        `입력 ${formatTokens(t.input)} · 출력 ${formatTokens(t.output)} · ` +
          `캐시 쓰기 ${formatTokens(t.cacheWrite)} · 캐시 읽기 ${formatTokens(t.cacheRead)}`,
      ),
    );
  }

  if (report.tokensLast5h) {
    node.append(statRow("최근 5시간", `${formatTokens(totalTokens(report.tokensLast5h))} 토큰`));
  }
  if (report.tokensLast7d) {
    node.append(statRow("최근 7일", `${formatTokens(totalTokens(report.tokensLast7d))} 토큰`));
  }
  if (report.hourlyTokens24h) {
    node.append(hourlyChart(report.hourlyTokens24h));
  }

  if (report.modelsToday?.length) {
    const list = el("ul", "models");
    for (const m of report.modelsToday) {
      const item = el("li");
      item.append(
        el("span", undefined, m.model),
        el("span", "muted", formatTokens(totalTokens(m.tokens))),
      );
      list.append(item);
    }
    node.append(list);
  }

  if (report.error) {
    node.append(el("div", "muted error", report.error));
  }

  return node;
}

async function refresh() {
  const container = document.querySelector<HTMLElement>("#cards");
  if (!container) return;

  try {
    const reports = await invoke<ProviderReport[]>("get_usage");
    container.replaceChildren(...reports.map(card));
  } catch (e) {
    container.replaceChildren(el("div", "muted error", `사용량을 가져오지 못했습니다: ${e}`));
  }
}

window.addEventListener("DOMContentLoaded", refresh);
// 창이 열릴 때마다 최신 값으로 갱신
window.addEventListener("focus", refresh);
