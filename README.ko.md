# Fitzroy

[English](README.md) | 한국어

Claude와 Codex 사용량을 한눈에 보여 주는 macOS 메뉴 막대 앱입니다.

아이콘은 파타고니아의 피츠로이산을 본떴습니다. 해가 뜰 때 봉우리가 붉게 물드는 모습을 "불타는 고구마"라고도 부르는데, Fitzroy는 이 모습을 빌려 왔습니다. 여유가 있을 때는 산이 평범한 낮 빛으로 보이고, 세션 사용량이 한도에 가까워질수록 봉우리가 금빛에서 짙은 노을빛으로 바뀝니다.
열심히 고구마를 구워봅시다...

아이콘을 누르면 창이 열립니다. 오른쪽 위 메뉴에서 **Claude**와 **Codex**를 골라 서로 바꿔 볼 수 있고, 메뉴 막대의 숫자는 고른 서비스를 따라갑니다. 쓰고 있는 플랜(예: **Claude Pro**, **Codex Go**)이 이름 옆에 표시됩니다.

- 한도 사용률과 재설정 시각
- 오늘, 최근 5시간, 최근 7일의 토큰 사용량과 모델별 내역
- 이번 주 날짜별 토큰 사용량

숫자마다 가져오는 곳이 다릅니다.

- **Claude 한도 사용률(%)** 은 Claude 계정 전체 기준입니다. 웹·데스크톱·모바일 Claude 채팅, Claude Code 등 플랜 한도에 들어가는 모든 사용이 모든 기기에서 합쳐집니다.
- **Claude 토큰 수** 는 이 Mac의 Claude Code 사용분만 셉니다. 터미널, IDE, Claude 데스크톱 앱 어디서 실행했든 포함됩니다. 채팅과, 다른 기기나 클라우드에서 쓴 Claude Code 세션은 포함되지 않습니다.

- **Codex 한도 사용률(%)** 은 Codex CLI의 로그인 정보로 ChatGPT 사용량 서비스에서 가져오므로 항상 최신입니다. Codex, Work, 워크스페이스 에이전트, Excel용 ChatGPT가 함께 쓰는 한도입니다. ChatGPT **Chat** 대화는 이 한도에 포함되지 않아서, 채팅만 해서는 값이 오르지 않습니다. 서비스에 연결하지 못하면 Codex 로그에 남은 마지막 값으로 대신하고, 그 값이 기록된 시각을 함께 보여 줍니다.
- **Codex 토큰 수** 는 이 Mac에 있는 Codex CLI의 세션 로그에서 읽습니다.

> [!IMPORTANT]
> Codex 지원은 한도가 30일 하나뿐인 ChatGPT **Go** 플랜에서만 만들고 시험했습니다. Plus와 Pro 플랜에는 5시간, 주간 같은 다른 한도가 있습니다. Fitzroy가 보여 줄 것으로 생각하지만 시험해 보지는 못했습니다. 이상하게 보이면 [이슈를 남겨 주세요](https://github.com/honeydragoneye/fitzroy/issues).

Gemini 지원은 이후 버전에서 추가할 예정입니다.

> [!NOTE]
> Fitzroy는 비공식 앱이며 Anthropic, OpenAI, Google과 관련이 없습니다. 공개되지 않은 API와 로컬 로그 형식을 사용하므로 언제든 동작하지 않을 수 있습니다.

## 필요 조건

- macOS 13 이상
- Apple 실리콘(M1 이상) Mac. Intel Mac은 아직 지원하지 않습니다.
- 채팅에서 쓰는 Claude와 같은 계정으로 로그인한 [Claude Code](https://docs.anthropic.com/en/docs/claude-code). Claude Code를 직접 쓸 필요는 없습니다. Fitzroy는 한도 사용률(채팅 사용분 포함)을 조회하려고 그 로그인 정보만 읽습니다. 터미널에서 `claude`를 실행해 한 번 로그인하세요. 토큰 수는 Claude Code의 로컬 로그에서 읽기 때문에, Claude Code를 쓰기 전까지는 비어 있습니다.
- Codex용(선택): ChatGPT 계정으로 로그인한 [Codex CLI](https://github.com/openai/codex). Fitzroy는 그 로그인 정보로 한도를 조회하고, 토큰 수는 로컬 세션 로그에서 읽습니다. 그래서 토큰 수는 Codex를 쓰기 전까지 비어 있습니다.

## 설치

1. [Releases](https://github.com/honeydragoneye/fitzroy/releases)에서 최신 `.dmg`를 내려받습니다.
2. `.dmg`를 열고 **Fitzroy**를 **응용 프로그램(Applications)** 폴더로 끌어다 놓습니다. 이전 Fitzroy가 이미 있으면 **대치**를 고릅니다.
3. **터미널**을 열고 아래 명령을 한 번 실행합니다. macOS가 앱을 열 수 있게 해 줍니다.

   ```sh
   xattr -dr com.apple.quarantine /Applications/Fitzroy.app
   ```

4. 응용 프로그램 폴더에서 Fitzroy를 엽니다.

이 앱은 Apple Developer ID로 서명되어 있지 않아서, macOS가 악성 소프트웨어인지 확인할 수 없다며 처음 열 때 막습니다. 3번이 그 차단을 풀어 줍니다. 터미널을 쓰고 싶지 않다면:

1. Fitzroy를 한 번 열어 보고 경고 창을 닫습니다. **휴지통으로 이동**은 고르지 마세요.
2. 바로 이어서 **시스템 설정 → 개인정보 보호 및 보안**을 엽니다.
3. 아래로 내려 Fitzroy 관련 메시지 옆의 **그래도 열기**를 누르고 확인합니다. 이 버튼은 경고가 뜬 뒤 약 한 시간 동안만 나타납니다.

Fitzroy는 메뉴 막대에서만 동작하고 Dock 아이콘이 없습니다. 종료하려면 창을 열고 오른쪽 위 메뉴 버튼을 눌러 **Fitzroy 종료**를 고릅니다.

## 업데이트

Fitzroy는 일주일에 한 번 GitHub에 최신 릴리스 버전을 물어봅니다. 계정 정보는 보내지 않습니다. 새 버전이 나오면 창 위쪽에 알림이 뜹니다. 알림을 누르면 릴리스 페이지가 열리고, 새 `.dmg`를 내려받아 Fitzroy를 응용 프로그램 폴더에 다시 끌어다 놓으면 기존 앱이 바뀝니다.

바로 확인하려면 메뉴에서 **업데이트 확인**을 고릅니다. 주간 확인을 끄려면 같은 메뉴에서 **업데이트 자동 확인**을 끕니다.

## 피드백

문제나 개선 의견을 보내려면 메뉴에서 **문제신고/개선사항 🕊️**를 고릅니다. Fitzroy가 메일 앱에 새 메일을 열고 짧은 양식과 진단 정보를 채워 넣습니다. 진단 정보는 앱과 macOS 버전, 갱신 주기, 한도 값이나 에러 메시지가 있는지 여부입니다. 토큰, 계정 정보, 대화 내용, 사용량 숫자는 절대 넣지 않습니다. Fitzroy가 메일을 직접 보내지는 않으니, 내용을 확인하고 고친 뒤 보내면 됩니다. myyseooo@gmail.com으로 직접 메일을 보내셔도 됩니다.

## 소스에서 빌드

[Rust](https://www.rust-lang.org/tools/install), [Node.js](https://nodejs.org/) 20.19 이상, Xcode Command Line Tools(`xcode-select --install`)가 필요합니다.

```sh
git clone https://github.com/honeydragoneye/fitzroy.git
cd fitzroy
npm install
npm run tauri build
```

앱은 `src-tauri/target/release/bundle/macos/Fitzroy.app`에, 디스크 이미지는 `src-tauri/target/release/bundle/dmg/`에 만들어집니다.

개발 모드로 실행하려면:

```sh
npm run tauri dev
```

## 라이선스

MIT. [LICENSE](LICENSE)를 참고하세요.
