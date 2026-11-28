# sumi 0.52.2

- **Task creation in chat.** Fixed AI requests failing before a plan could be returned. Requests containing multiple tasks can produce separate task actions while the app retains its local validation, authorization and confirmation checks.
- **Clearer chat errors.** AI connection and response failures now show safe, relevant messages instead of a misleading save error.
- **Companion mouse handling.** Temporary cursor and window probe failures now restore mouse input and retry before requiring a restart.

Available for Windows x64 and macOS Apple Silicon (macOS 14.5 or later). Provider credentials remain runtime-only; trusted updater keys and the reviewed macOS app-signing identity are unchanged.
