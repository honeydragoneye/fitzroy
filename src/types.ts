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
  // 최근 24시간의 시간별 토큰 합계. 오래된 것부터 24칸.
  hourlyTokens24h: number[] | null;
  error: string | null;
}
