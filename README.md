# Fitzroy

A macOS menu bar app that shows your Claude and Codex usage at a glance.

The icon is modeled on Mount Fitz Roy in Patagonia. At sunrise its granite peaks glow red, which Korean hikers call the "burning sweet potato." Fitzroy borrows that look. While you have plenty of room, the mountain sits in plain daylight. As your session usage gets close to its limit, the peaks turn from gold to a deep sunset red.

Click the icon to open a panel. Choose **Claude** or **Codex** from the menu in the top right to switch between them. The number in the menu bar follows the one you choose. The plan you are on (for example **Claude Pro** or **Codex Go**) is shown next to the name.

- Usage limits with reset times
- Token usage today, over the last 5 hours, and over the last 7 days, with a breakdown by model
- Daily token usage for this week

The numbers come from different places:

- **Claude limit usage (%)** is your whole Claude account: chats in Claude on the web, desktop, and mobile, Claude Code, and any other Claude use that counts toward your plan's limits, on every device.
- **Claude token counts** cover Claude Code on this Mac only, wherever you run it: the terminal, your IDE, or the Claude desktop app. They don't include chats or Claude Code sessions on other devices or in the cloud.
- **Codex** is read from the Codex CLI's local session logs, for limits and token counts alike. It covers Codex CLI use on this Mac only, not ChatGPT chats. The values update only when you use the Codex CLI, and the panel shows when they were last updated.

> [!IMPORTANT]
> Codex support was built and tested on the ChatGPT **Go** plan only, which has a single 30-day limit. Plus and Pro plans have different limits, such as 5-hour and weekly windows. Fitzroy should show them, but this has not been tested. If they look wrong, please [open an issue](https://github.com/honeydragoneye/fitzroy/issues).

Support for Gemini is planned for a future version.

> [!NOTE]
> Fitzroy is an unofficial app. It is not affiliated with Anthropic, OpenAI, or Google. It relies on undocumented APIs and local log formats, so it may stop working at any time.

## Requirements

- macOS 13 or later
- A Mac with Apple silicon (M1 or later). Intel Macs are not supported yet.
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code), installed and logged in. Fitzroy reads the usage limits with Claude Code's login and reads token counts from Claude Code's local logs.
- For Codex: the [Codex CLI](https://github.com/openai/codex), logged in with a ChatGPT account. Fitzroy reads only its local session logs and sends nothing over the network for Codex.

## Install

1. Download the latest `.dmg` from [Releases](https://github.com/honeydragoneye/fitzroy/releases).
2. Open the `.dmg` and drag **Fitzroy** into **Applications**. If an older Fitzroy is already there, choose **Replace**.
3. Open **Terminal** and run this once. It lets macOS open the app:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Fitzroy.app
   ```

4. Open Fitzroy from Applications.

The app is not signed with an Apple Developer ID, so macOS says it can't check Fitzroy for malicious software and blocks it the first time. Step 3 removes that block. If you would rather not use Terminal:

1. Try to open Fitzroy once, and close the warning. Don't choose **Move to Trash**.
2. Right after that, open **System Settings → Privacy & Security**.
3. Scroll down and click **Open Anyway** next to the message about Fitzroy, then confirm. The button appears only for about an hour after the warning.

Fitzroy runs only in the menu bar and has no Dock icon. To quit it, open the panel, click the menu button in the top right, and choose **Quit Fitzroy**.

## Updates

Once a week, Fitzroy asks GitHub for the latest release version. It sends no account information. When a newer version is out, a notice appears at the top of the panel. Click it to open the release page, download the new `.dmg`, and drag Fitzroy into Applications again to replace the old copy.

To check right away, choose **Check for Updates** from the menu. To stop the weekly check, turn off **Check for Updates Automatically** in the same menu.

## Feedback

To report a problem, choose **Report a Problem 🕊️** from the menu. Fitzroy opens a new message in your mail app with a short form and some diagnostic details filled in: the app and macOS versions, the refresh interval, and whether Fitzroy has limit values or an error message. It never includes tokens, account details, conversation content, or usage numbers. Fitzroy does not send the message itself. You can review and edit it before you send it. You can also write to myyseooo@gmail.com directly.

## Build from source

You need [Rust](https://www.rust-lang.org/tools/install), [Node.js](https://nodejs.org/) 20.19 or later, and the Xcode Command Line Tools (`xcode-select --install`).

```sh
git clone https://github.com/honeydragoneye/fitzroy.git
cd fitzroy
npm install
npm run tauri build
```

The app is created in `src-tauri/target/release/bundle/macos/Fitzroy.app`, and the disk image in `src-tauri/target/release/bundle/dmg/`.

To run it in development mode:

```sh
npm run tauri dev
```

## License

MIT. See [LICENSE](LICENSE).
