export interface TokenUsage {
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
}

export interface ModelUsage {
  model: string;
  tokens: TokenUsage;
}

export interface DailyTokens {
  // 그날이 시작하는 시각(로컬 자정)
  dayStart: number;
  tokens: number;
}

export interface LimitWindow {
  // "현재 세션", "이번 주", "30일" 처럼 화면에 그대로 쓰는 이름
  label: string;
  percent: number;
  resetsAt: number | null;
  // 가장 최근에 지난 재설정 시각. 다음 재설정 시각을 모를 때(사용률 0%) 대신 보여준다.
  lastResetAt: number | null;
}

// src-tauri/src/updates.rs 의 Release
export interface Release {
  version: string;
  url: string;
}

// 메뉴에서 "업데이트 확인…"을 골랐을 때의 진행 상황
export type UpdateStatus =
  | { state: "checking" }
  | { state: "upToDate" }
  | { state: "available"; version: string }
  | { state: "failed" };

// src-tauri/src/providers/mod.rs 의 ProviderReport 와 같은 형식. 시각은 Unix epoch 밀리초.
export interface ProviderReport {
  id: string;
  name: string;
  // 이름을 눌러 사용량 화면을 열 수 있는지
  hasUsagePage: boolean;
  limits: LimitWindow[];
  // 한도 값이 언제 기준인지. 로그에서 읽어 사용할 때만 갱신되는 서비스에서 쓴다.
  limitsAsOf: number | null;
  tokensToday: TokenUsage | null;
  tokensLast5h: TokenUsage | null;
  tokensLast7d: TokenUsage | null;
  modelsToday: ModelUsage[] | null;
  // 이번 주(일요일~토요일)의 날짜별 토큰 합계. 아직 오지 않은 날은 0.
  dailyTokensThisWeek: DailyTokens[] | null;
  error: string | null;
}
