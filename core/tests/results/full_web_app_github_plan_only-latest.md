# E2E Test Result: full-web-app-github-plan-only

**Status:** ❌ FAILED

**Date:** 2026-02-04

**Time:** 06:50:52 UTC

## Summary

| Metric | Value |
|--------|-------|
| Fixture | `full-web-app-github-plan-only` |
| Spawn Success | false |
| Overall Passed | false |
| Repository | [`epiphytic/e2e-full-web-app-github-plan-only-bf4ede83`](https://github.com/epiphytic/e2e-full-web-app-github-plan-only-bf4ede83) |

## Error

```
CruiseRunner failed: cruise-control error: Gemini reviewer failed in Security phase. Exit code: Some(1). Stderr: YOLO mode is enabled. All tool calls will be automatically approved.
Loaded cached credentials.
YOLO mode is enabled. All tool calls will be automatically approved.
Loading extension: gemini-cli-git
Loading extension: gemini-cli-ralph
Loading extension: gemini-deep-research
Loading extension: gemini-plan-commands
MCP server 'browseros': HTTP connection failed, attempting SSE fallback...
MCP server 'browseros': SSE fallback also failed.
Error during discovery for MCP server 'browseros': fetch failedHook registry initialized with 0 hook entries
Server 'gemini-deep-research' supports tool updates. Listening for changes...
Error when talking to Gemini API Full report available at: /var/folders/ps/kk0xycp9121dn6bpz_wrgp_80000gn/T/gemini-client-error-Turn.run-sendMessageStream-2026-02-04T06-50-52-455Z.json TerminalQuotaError: You have exhausted your capacity on this model. Your quota will reset after 11h35m53s.
    at classifyGoogleError (file:///Users/liam.helmer/node_modules/@google/gemini-cli-core/dist/src/utils/googleQuotaErrors.js:214:28)
    at retryWithBackoff (file:///Users/liam.helmer/node_modules/@google/gemini-cli-core/dist/src/utils/retry.js:130:37)
    at process.processTicksAndRejections (node:internal/process/task_queues:105:5)
    at async GeminiChat.makeApiCallAndProcessStream (file:///Users/liam.helmer/node_modules/@google/gemini-cli-core/dist/src/core/geminiChat.js:421:32)
    at async GeminiChat.streamWithRetries (file:///Users/liam.helmer/node_modules/@google/gemini-cli-core/dist/src/core/geminiChat.js:253:40)
    at async Turn.run (file:///Users/liam.helmer/node_modules/@google/gemini-cli-core/dist/src/core/turn.js:66:30)
    at async GeminiClient.processTurn (file:///Users/liam.helmer/node_modules/@google/gemini-cli-core/dist/src/core/client.js:458:26)
    at async GeminiClient.sendMessageStream (file:///Users/liam.helmer/node_modules/@google/gemini-cli-core/dist/src/core/client.js:554:20)
    at async file:///Users/liam.helmer/node_modules/@google/gemini-cli/dist/src/nonInteractiveCli.js:177:34
    at async main (file:///Users/liam.helmer/node_modules/@google/gemini-cli/dist/src/gemini.js:474:9) {
  cause: {
    code: 429,
    message: 'You have exhausted your capacity on this model. Your quota will reset after 11h35m53s.',
    details: [ [Object], [Object] ]
  },
  retryDelayMs: 41753555.459131
}

```

