import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { formatDay, formatPastTime, formatResetTime, formatTokens, totalTokens } from "./format";
import type { DailyTokens, ProviderReport } from "./types";

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
  lastResetAt: number | null,
): HTMLElement | null {
  if (percent === null) return null;

  const row = el("div", "limit");
  const head = el("div", "limit-head");
  head.append(el("span", "limit-label", label));
  if (resetsAt !== null) {
    head.append(el("span", "muted", `${formatResetTime(resetsAt)}에 재설정`));
  } else if (lastResetAt !== null) {
    // 재설정 뒤 아직 쓰지 않았으면 다음 재설정 시각이 없어서, 지난 재설정 시각을 보여준다.
    head.append(el("span", "muted", `${formatPastTime(lastResetAt)}에 재설정됨`));
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

const WEEKDAY_LABELS = ["S", "M", "T", "W", "T", "F", "S"];

// 이번 주(일요일~토요일) 날짜별 막대. 막대에 커서를 올리면 그날의 날짜와 사용량이 뜬다.
function weekChart(days: DailyTokens[]): HTMLElement {
  const chart = el("div", "chart");
  const max = Math.max(...days.map((d) => d.tokens), 1);
  const now = Date.now();
  const todayIndex = days.reduce((last, day, index) => (day.dayStart <= now ? index : last), -1);

  days.forEach((day, index) => {
    const column = el("div", "chart-column");
    if (index === todayIndex) column.classList.add("chart-today");
    if (index > todayIndex) column.classList.add("chart-future");

    const track = el("div", "chart-track");
    const bar = el("div", "chart-bar");
    bar.style.height = `${Math.max(day.tokens > 0 ? 6 : 0, (day.tokens / max) * 100)}%`;
    track.append(bar);

    column.append(track, el("div", "chart-label", WEEKDAY_LABELS[index % 7]));

    if (index <= todayIndex) {
      const tip = el(
        "div",
        "chart-tip",
        `${formatDay(day.dayStart)} · ${formatTokens(day.tokens)} 토큰`,
      );
      // 양 끝 막대의 말풍선이 카드 밖으로 나가지 않게 정렬을 바꾼다.
      if (index < 2) tip.classList.add("chart-tip-start");
      else if (index >= days.length - 2) tip.classList.add("chart-tip-end");
      column.append(tip);
    }

    chart.append(column);
  });
  return chart;
}

function card(report: ProviderReport): HTMLElement {
  const node = el("section", "card");
  const title = el("button", "card-title", report.name);
  title.title = `${report.name} 사용량 화면 열기`;
  title.addEventListener("click", () => {
    void invoke("open_usage_page", { id: report.id }).catch(() => {});
  });
  node.append(title);

  const session = limitRow(
    "현재 세션",
    report.sessionPercent,
    report.sessionResetsAt,
    report.sessionLastResetAt,
  );
  const weekly = limitRow(
    "이번 주",
    report.weeklyPercent,
    report.weeklyResetsAt,
    report.weeklyLastResetAt,
  );
  if (session) node.append(session);
  if (weekly) node.append(weekly);

  if (report.tokensToday) {
    const t = report.tokensToday;
    node.append(
      el("div", "tokens", `오늘 ${formatTokens(totalTokens(t))} 토큰`),
      el("div", "muted", `입력 ${formatTokens(t.input)} · 출력 ${formatTokens(t.output)}`),
      el(
        "div",
        "muted",
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
  if (report.dailyTokensThisWeek?.length) {
    node.append(weekChart(report.dailyTokensThisWeek));
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

function render(reports: ProviderReport[]) {
  document.querySelector<HTMLElement>("#cards")?.replaceChildren(...reports.map(card));
}

const PANEL_WIDTH = 300;
const PANEL_MAX_HEIGHT = 640;

// 창 높이를 내용에 맞춘다. 내용이 바뀔 때마다 다시 맞춘다.
function fitWindowToContent(container: HTMLElement) {
  let lastHeight = 0;
  new ResizeObserver(() => {
    const height = Math.min(PANEL_MAX_HEIGHT, Math.ceil(container.getBoundingClientRect().height));
    if (height === 0 || height === lastHeight) return;
    lastHeight = height;
    void getCurrentWindow().setSize(new LogicalSize(PANEL_WIDTH, height));
  }).observe(container);
}

// 메뉴에서 "Fitzroy에 관하여"를 고르면 카드 대신 앱 정보를 보여준다.
async function setUpAbout() {
  const about = document.querySelector<HTMLElement>("#about");
  const cards = document.querySelector<HTMLElement>("#cards");
  if (!about || !cards) return;

  const show = (visible: boolean) => {
    about.hidden = !visible;
    cards.hidden = visible;
  };

  const version = about.querySelector<HTMLElement>("#about-version");
  if (version) version.textContent = `v${await getVersion()}`;

  about.querySelector("#about-close")?.addEventListener("click", () => show(false));
  about.querySelector("#about-repository")?.addEventListener("click", () => {
    void invoke("open_repository").catch(() => {});
  });
  await listen("show-about", () => show(true));
}

// 머리글의 메뉴 버튼을 누르면 버튼 바로 아래에 앱 메뉴(종료 등)를 띄운다.
function setUpAppMenu() {
  const button = document.querySelector<HTMLButtonElement>("#app-menu");
  if (!button) return;

  button.addEventListener("click", () => {
    const rect = button.getBoundingClientRect();
    void invoke("show_app_menu", { x: rect.left, y: rect.bottom + 4 }).catch(() => {});
  });
}

window.addEventListener("DOMContentLoaded", async () => {
  const container = document.querySelector<HTMLElement>("#app");
  if (container) fitWindowToContent(container);
  setUpAppMenu();

  // 값은 뒤에서 주기적으로 갱신되어 이벤트로 온다. 그 전까지는 마지막으로 받아 둔 값을 보여준다.
  await listen<ProviderReport[]>("usage-updated", (event) => render(event.payload));
  render(await invoke<ProviderReport[]>("get_usage"));
  await setUpAbout();
});
