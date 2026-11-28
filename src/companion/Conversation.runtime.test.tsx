// @vitest-environment jsdom
import { queryClient } from "../data/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Conversation } from "./Conversation";
import { defaultCompanionView } from "./types";
import { aiApi, defaultAiPreferences } from "../ai/api";
import { coreApi } from "../core/api";
import { recordVoice } from "../core/media";
import { StrictMode } from "react";
vi.mock("../api/tauri", () => ({
  desktop: true,
  api: {},
  safeUrl: () => null,
}));
vi.mock("../api/platform", () => ({ isMac: true }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("../ai/api", async (original) => ({
  ...(await original<typeof import("../ai/api")>()),
  aiApi: { history: vi.fn(), preferences: vi.fn(), send: vi.fn() },
}));
vi.mock("../core/api", () => ({
  coreApi: {
    transcribe: vi.fn(),
    correctVoiceDraft: vi.fn(),
    voiceLanguages: vi.fn(),
    prepareVoice: vi.fn(async () => {}),
  },
}));
vi.mock("../core/media", () => ({
  recordVoice: vi.fn(),
  readAttachment: vi.fn(),
}));
beforeEach(() => {
  queryClient.invalidate(undefined, true);
  vi.clearAllMocks();
  vi.mocked(aiApi.history).mockResolvedValue([]);
  vi.mocked(aiApi.preferences).mockResolvedValue(defaultAiPreferences);
  vi.mocked(aiApi.send).mockResolvedValue({ message: "Done", resources: [] });
  vi.mocked(coreApi.voiceLanguages).mockResolvedValue(["en-US"]);
  vi.mocked(coreApi.transcribe).mockResolvedValue("Add an outline step");
  vi.mocked(recordVoice).mockResolvedValue({
    stop: vi.fn().mockResolvedValue("fixture"),
    cancel: vi.fn(),
    setMuted: vi.fn(),
  });
});
afterEach(cleanup);
it("a failed model download gives a safe retry message and never opens the microphone", async () => {
  const message =
    "The speech model could not download. Check your connection and try again.";
  vi.mocked(coreApi.prepareVoice).mockRejectedValueOnce(message);
  render(
    <Conversation
      view={defaultCompanionView}
      goal={null}
      plan={null}
      compact
      mode="voice"
      onChanged={async () => {}}
      onState={() => {}}
    />,
  );
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain(message),
  );
  expect(recordVoice).not.toHaveBeenCalled();
  expect(aiApi.send).not.toHaveBeenCalled();
});
it("prepares the local model before opening the microphone and cancels a pending start", async () => {
  let finish!: () => void;
  vi.mocked(coreApi.prepareVoice).mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  const props = {
    view: defaultCompanionView,
    goal: null,
    plan: null,
    compact: true,
    onChanged: async () => {},
    onState: () => {},
  };
  const { rerender } = render(<Conversation {...props} mode="voice" />);
  await waitFor(() => expect(coreApi.prepareVoice).toHaveBeenCalled());
  expect(recordVoice).not.toHaveBeenCalled();
  expect(screen.getByText(/Preparing on-device speech/)).toBeTruthy();
  rerender(<Conversation {...props} mode="text" />);
  await act(async () => finish());
  expect(recordVoice).not.toHaveBeenCalled();
});
it("local voice preparation survives Strict Mode without opening two microphones", async () => {
  render(
    <StrictMode>
      <Conversation
        view={defaultCompanionView}
        goal={null}
        plan={null}
        compact
        mode="voice"
        onChanged={async () => {}}
        onState={() => {}}
      />
    </StrictMode>,
  );
  await screen.findByRole("button", { name: "Mute microphone" });
  expect(recordVoice).toHaveBeenCalledOnce();
  expect(screen.getByText(/Listening. Pauses are fine/)).toBeTruthy();
});
it("opens the bubble's short draft for editing without sending it automatically", async () => {
  const draft = "I want to talk about “My list of companies”.";
  render(
    <Conversation
      view={{ ...defaultCompanionView, seed: draft }}
      goal={null}
      plan={null}
      compact
      mode="text"
      onChanged={async () => {}}
      onState={() => {}}
    />,
  );
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
    draft,
  );
  expect(aiApi.send).not.toHaveBeenCalled();
  await userEvent.type(screen.getByRole("textbox"), " Remove its deadline.");
  await userEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() =>
    expect(aiApi.send).toHaveBeenCalledWith(
      `${draft} Remove its deadline.`,
      null,
      null,
      null,
      false,
      expect.any(String),
      null,
      null,
      false,
    ),
  );
});
function show() {
  render(
    <Conversation
      view={defaultCompanionView}
      goal={null}
      plan={null}
      conversationId={42}
      onChanged={async () => {}}
      onState={() => {}}
    />,
  );
}
async function dictate() {
  await userEvent.click(
    screen.getByRole("button", { name: "Start voice recording" }),
  );
  await screen.findByRole("button", { name: "Stop voice recording" });
  await userEvent.click(
    screen.getByRole("button", { name: "Stop voice recording" }),
  );
  await waitFor(() =>
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
      "Add an outline step",
    ),
  );
}
it("routes a locally transcribed message as voice, then an ordinary message as text", async () => {
  show();
  await dictate();
  await userEvent.keyboard("{Enter}");
  await waitFor(() =>
    expect(aiApi.send).toHaveBeenNthCalledWith(
      1,
      "Add an outline step",
      null,
      null,
      42,
      true,
      expect.any(String),
      null,
      null,
      false,
    ),
  );
  await userEvent.type(screen.getByRole("textbox"), "List my goals{Enter}");
  await waitFor(() =>
    expect(aiApi.send).toHaveBeenNthCalledWith(
      2,
      "List my goals",
      null,
      null,
      42,
      false,
      expect.any(String),
      null,
      null,
      false,
    ),
  );
});
it("preserves the input source and chat when a message waits behind another reply", async () => {
  let finish!: (reply: { message: string; resources: [] }) => void;
  vi.mocked(aiApi.send).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  show();
  await dictate();
  await userEvent.keyboard("{Enter}");
  await waitFor(() => expect(aiApi.send).toHaveBeenCalledTimes(1));
  await userEvent.type(screen.getByRole("textbox"), "List my goals{Enter}");
  expect(aiApi.send).toHaveBeenCalledTimes(1);
  await act(async () => finish({ message: "Step added", resources: [] }));
  await waitFor(() =>
    expect(aiApi.send).toHaveBeenNthCalledWith(
      2,
      "List my goals",
      null,
      null,
      42,
      false,
      expect.any(String),
      null,
      null,
      false,
    ),
  );
});

it.each(["", "   "])(
  "does not send an empty transcript (%j) to AI",
  async (transcript) => {
    vi.mocked(coreApi.transcribe).mockResolvedValue(transcript);
    show();
    await userEvent.click(
      screen.getByRole("button", { name: "Start voice recording" }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Stop voice recording" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain(
        "No speech was recognized",
      ),
    );
    expect(aiApi.send).not.toHaveBeenCalled();
  },
);
it("preserves the typed draft after a local STT failure", async () => {
  vi.mocked(coreApi.transcribe).mockRejectedValue(
    new Error("Local recognition failed"),
  );
  show();
  await userEvent.type(screen.getByRole("textbox"), "Keep this draft");
  await userEvent.click(
    screen.getByRole("button", { name: "Start voice recording" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "Stop voice recording" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain(
      "sumi couldn't finish this request. Check the chat and your goals before retrying.",
    ),
  );
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
    "Keep this draft",
  );
  expect(aiApi.send).not.toHaveBeenCalled();
});
it.each([
  "sumi's AI provider is temporarily unavailable. Your message is saved; try again in a moment.",
  "sumi's AI provider rejected this request. Your message is saved. Try a new chat or update sumi before retrying.",
  "AI assistance is paused. Enable it in Settings → Integrations / AI to continue.",
])("shows the safe backend chat error: %s", async (message) => {
  vi.mocked(aiApi.send).mockRejectedValueOnce(message);
  show();
  await userEvent.type(screen.getByRole("textbox"), "Create a task{Enter}");
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain(message),
  );
  expect(
    screen.queryByText("Couldn't save that change. Please try again."),
  ).toBeNull();
});
it("explains microphone permission failure and permits typed commands", async () => {
  vi.mocked(recordVoice).mockRejectedValueOnce(
    new DOMException("Denied", "NotAllowedError"),
  );
  show();
  await userEvent.click(
    screen.getByRole("button", { name: "Start voice recording" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain(
      "Microphone access was denied",
    ),
  );
  expect(coreApi.transcribe).not.toHaveBeenCalled();
  await userEvent.type(screen.getByRole("textbox"), "List goals{Enter}");
  await waitFor(() =>
    expect(aiApi.send).toHaveBeenCalledWith(
      "List goals",
      null,
      null,
      42,
      false,
      expect.any(String),
      null,
      null,
      false,
    ),
  );
});
it("does not start a microphone or cloud fallback if local STT is unsupported", async () => {
  vi.mocked(coreApi.voiceLanguages).mockResolvedValueOnce([]);
  show();
  await userEvent.click(
    screen.getByRole("button", { name: "Start voice recording" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain(
      "Local speech recognition is unavailable",
    ),
  );
  expect(recordVoice).not.toHaveBeenCalled();
  expect(coreApi.transcribe).not.toHaveBeenCalled();
  expect(aiApi.send).not.toHaveBeenCalled();
});
it("routes raw audio only to local STT and sends only the transcript to the agent", async () => {
  const audio = new Uint8Array([82, 73, 70, 70, 111, 222]);
  vi.mocked(recordVoice).mockResolvedValueOnce({
    stop: vi.fn().mockResolvedValue(audio),
    cancel: vi.fn(),
    setMuted: vi.fn(),
  });
  show();
  await dictate();
  expect(coreApi.transcribe).toHaveBeenCalledWith(audio);
  expect(aiApi.send).not.toHaveBeenCalled();
  await userEvent.keyboard("{Enter}");
  await waitFor(() =>
    expect(aiApi.send).toHaveBeenCalledWith(
      "Add an outline step",
      null,
      null,
      42,
      true,
      expect.any(String),
      null,
      null,
      false,
    ),
  );
  expect(
    vi
      .mocked(aiApi.send)
      .mock.calls[0].every((value: unknown) => !(value instanceof Uint8Array)),
  ).toBe(true);
});
it("discards recognition when the compact voice interface is closed before STT finishes", async () => {
  let complete!: (value: string) => void;
  vi.mocked(coreApi.transcribe).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  );
  const onClose = vi.fn();
  const props = {
    view: defaultCompanionView,
    goal: null,
    plan: null,
    compact: true,
    onChanged: async () => {},
    onState: () => {},
    onClose,
  };
  const { rerender } = render(<Conversation {...props} mode="voice" />);
  await waitFor(() => expect(recordVoice).toHaveBeenCalled());
  await userEvent.click(
    screen.getByRole("button", { name: "Stop and transcribe" }),
  );
  await waitFor(() => expect(coreApi.transcribe).toHaveBeenCalled());
  await userEvent.click(screen.getByRole("button", { name: "Close voice" }));
  expect(onClose).toHaveBeenCalled();
  rerender(<Conversation {...props} mode="text" />);
  await act(async () => complete("Delete all goals"));
  expect(aiApi.send).not.toHaveBeenCalled();
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
});
it("rejects oversized speech instead of keeping a truncated command", async () => {
  vi.mocked(coreApi.transcribe).mockResolvedValueOnce(
    "Delete the goal ".repeat(300),
  );
  render(
    <Conversation
      view={defaultCompanionView}
      goal={null}
      plan={null}
      compact
      mode="voice"
      onChanged={async () => {}}
      onState={() => {}}
    />,
  );
  await waitFor(() => expect(recordVoice).toHaveBeenCalled());
  await userEvent.click(
    screen.getByRole("button", { name: "Stop and transcribe" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain(
      "transcript is too long",
    ),
  );
  expect(aiApi.send).not.toHaveBeenCalled();
});

it("keeps desktop dictation through pauses and lets the person review it before sending", async () => {
  const transcript =
    "Find the job listing. Then add a one-hour call with my friend.";
  vi.mocked(coreApi.transcribe).mockResolvedValueOnce(transcript);
  const onMode = vi.fn();
  const props = {
    view: defaultCompanionView,
    goal: null,
    plan: null,
    compact: true,
    onChanged: async () => {},
    onState: () => {},
    onMode,
  };
  const { rerender } = render(<Conversation {...props} mode="voice" />);
  await waitFor(() => expect(recordVoice).toHaveBeenCalled());
  // No silence callback can stop a list while the person pauses to think.
  expect(vi.mocked(recordVoice).mock.calls[0]).toEqual([]);
  expect(coreApi.transcribe).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("button", { name: "Stop and transcribe" }),
  );
  await waitFor(() => expect(onMode).toHaveBeenCalledWith("text"));
  rerender(<Conversation {...props} mode="text" />);
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
    transcript,
  );
  expect(aiApi.send).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Send message" }));
  await waitFor(() =>
    expect(aiApi.send).toHaveBeenCalledWith(
      transcript,
      null,
      null,
      null,
      true,
      expect.any(String),
      null,
      null,
      false,
    ),
  );
});

it("counts the separator when appending speech so the last character is never truncated", async () => {
  vi.mocked(coreApi.transcribe).mockResolvedValueOnce("end");
  show();
  const draft = "a".repeat(3997);
  fireEvent.change(screen.getByRole("textbox"), { target: { value: draft } });
  await userEvent.click(
    screen.getByRole("button", { name: "Start voice recording" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "Stop voice recording" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("alert").textContent).toContain(
      "transcript is too long",
    ),
  );
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
    draft,
  );
  expect(aiApi.send).not.toHaveBeenCalled();
});

it("the 60-second recording limit produces a draft, never an automatic message", async () => {
  render(
    <Conversation
      view={defaultCompanionView}
      goal={null}
      plan={null}
      compact
      mode="voice"
      onChanged={async () => {}}
      onState={() => {}}
    />,
  );
  await screen.findByRole("button", { name: "Mute microphone" });
  vi.useFakeTimers();
  try {
    // The timer was scheduled before fake timers; replace it with a new recording.
    fireEvent.click(screen.getByRole("button", { name: "Switch to text" }));
    fireEvent.click(screen.getByRole("button", { name: "Start recording" }));
    await act(async () => {});
    await act(async () => vi.advanceTimersByTimeAsync(60000));
    expect(coreApi.transcribe).toHaveBeenCalledOnce();
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
      "Add an outline step",
    );
    expect(aiApi.send).not.toHaveBeenCalled();
  } finally {
    vi.useRealTimers();
  }
});

it("shows corrected dictation before editing, keeps the original reversible and never auto-sends", async () => {
  vi.mocked(aiApi.preferences).mockResolvedValue({
    ...defaultAiPreferences,
    voice_text_cleanup: true,
  });
  const raw = "Change the color and go to safety eye keys";
  const cleaned = "Rename the saved AI keys goal";
  vi.mocked(coreApi.transcribe).mockResolvedValueOnce(raw);
  let finish!: (value: {
    text: string;
    corrected: boolean;
    unavailable: boolean;
  }) => void;
  vi.mocked(coreApi.correctVoiceDraft).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  show();
  await userEvent.type(screen.getByRole("textbox"), "First note. ");
  await userEvent.click(
    screen.getByRole("button", { name: "Start voice recording" }),
  );
  await screen.findByRole("button", { name: "Stop voice recording" });
  await userEvent.click(
    screen.getByRole("button", { name: "Stop voice recording" }),
  );
  await screen.findByText("Tidying your words…");
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
    "First note. ",
  );
  expect(coreApi.correctVoiceDraft).toHaveBeenCalledWith(raw);
  expect(aiApi.send).not.toHaveBeenCalled();
  await act(async () =>
    finish({ text: cleaned, corrected: true, unavailable: false }),
  );
  await waitFor(() =>
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
      "First note.  " + cleaned,
    ),
  );
  await userEvent.click(screen.getByRole("button", { name: "Use original" }));
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
    "First note.  " + raw,
  );
  expect(aiApi.send).not.toHaveBeenCalled();
});
it("falls back to the original draft when cleanup fails without showing provider details", async () => {
  vi.mocked(aiApi.preferences).mockResolvedValue({
    ...defaultAiPreferences,
    voice_text_cleanup: true,
  });
  vi.mocked(coreApi.correctVoiceDraft).mockRejectedValueOnce(
    "private-cleanup-provider-detail",
  );
  show();
  await dictate();
  await screen.findByText(/original transcript is ready to edit/);
  expect(document.body.textContent).not.toContain(
    "private-cleanup-provider-detail",
  );
  expect(
    queryClient
      .getDiagnostics()
      .some((row) => row.detail.includes("private-cleanup-provider-detail")),
  ).toBe(true);
  expect(aiApi.send).not.toHaveBeenCalled();
});
it("does not upload a transcript after voice is closed during local recognition", async () => {
  vi.mocked(aiApi.preferences).mockResolvedValue({
    ...defaultAiPreferences,
    voice_text_cleanup: true,
  });
  let finish!: (value: string) => void;
  vi.mocked(coreApi.transcribe).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const props = {
    view: defaultCompanionView,
    goal: null,
    plan: null,
    compact: true,
    onChanged: async () => {},
    onState: () => {},
    onClose: () => {},
  };
  const { rerender } = render(<Conversation {...props} mode="voice" />);
  await waitFor(() => expect(recordVoice).toHaveBeenCalled());
  await userEvent.click(
    screen.getByRole("button", { name: "Stop and transcribe" }),
  );
  await waitFor(() => expect(coreApi.transcribe).toHaveBeenCalled());
  await userEvent.click(screen.getByRole("button", { name: "Close voice" }));
  rerender(<Conversation {...props} mode="text" />);
  await act(async () => finish("private pending dictation"));
  expect(coreApi.correctVoiceDraft).not.toHaveBeenCalled();
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
});
it("discards a late corrected draft when voice is closed", async () => {
  vi.mocked(aiApi.preferences).mockResolvedValue({
    ...defaultAiPreferences,
    voice_text_cleanup: true,
  });
  let finish!: (value: {
    text: string;
    corrected: boolean;
    unavailable: boolean;
  }) => void;
  vi.mocked(coreApi.correctVoiceDraft).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const props = {
    view: defaultCompanionView,
    goal: null,
    plan: null,
    compact: true,
    onChanged: async () => {},
    onState: () => {},
    onClose: () => {},
  };
  const { rerender } = render(<Conversation {...props} mode="voice" />);
  await waitFor(() => expect(recordVoice).toHaveBeenCalled());
  await userEvent.click(
    screen.getByRole("button", { name: "Stop and transcribe" }),
  );
  await screen.findByText("Tidying your words…");
  await userEvent.click(screen.getByRole("button", { name: "Close voice" }));
  rerender(<Conversation {...props} mode="text" />);
  await act(async () =>
    finish({ text: "Corrected goal", corrected: true, unavailable: false }),
  );
  expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
  expect(aiApi.send).not.toHaveBeenCalled();
});
