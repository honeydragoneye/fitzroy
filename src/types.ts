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

// src-tauri/src/providers/mod.rs 의 ProviderReport 와 같은 형식. 시각은 Unix epoch 밀리초.
export interface ProviderReport {
  id: string;
  name: string;
  sessionPercent: number | null;
  sessionResetsAt: number | null;
  weeklyPercent: number | null;
  weeklyResetsAt: number | null;
  tokensToday: TokenUsage | null;
  tokensLast5h: TokenUsage | null;
  tokensLast7d: TokenUsage | null;
  modelsToday: ModelUsage[] | null;
  // 이번 주(일요일~토요일)의 날짜별 토큰 합계. 아직 오지 않은 날은 0.
  dailyTokensThisWeek: DailyTokens[] | null;
  error: string | null;
}
