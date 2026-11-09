import { isMac } from "../api/platform";
import { useEffect, useRef, useState } from "react";
import { ArrowUp, FileUp, Mic, Square, X, Plus } from "lucide-react";
import { desktop } from "../api/tauri";
import { coreApi } from "./api";
import { readAttachment, recordVoice, textGoals } from "./media";
import type { GoalDraft, ImageInput, ImportProposal } from "./types";

export function Composer({
  onSave,
  onDirty,
  disabled = false,
  importMode = false,
  mock = false,
}: {
  onSave: (drafts: GoalDraft[], batch: string) => Promise<void>;
  onDirty?: (dirty: boolean) => void;
  disabled?: boolean;
  importMode?: boolean;
  mock?: boolean;
}) {
  const [text, setText] = useState("");
  const [images, setImages] = useState<ImageInput[]>([]);
  const [files, setFiles] = useState<string[]>([]);
  const [proposal, setProposal] = useState<ImportProposal | null>(null);
  const [batch, setBatch] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [recording, setRecording] = useState(false);
  const [voiceLanguage, setVoiceLanguage] = useState("auto");
  const [voiceLanguages, setVoiceLanguages] = useState<string[] | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const recorder = useRef<Awaited<ReturnType<typeof recordVoice>> | null>(null);
  const timeout = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dirty = !!(text.trim() || images.length || proposal || recording);
  useEffect(() => {
    onDirty?.(dirty);
  }, [dirty, onDirty]);
  useEffect(() => {
    if (desktop && importMode)
      void coreApi
        .voiceLanguages()
        .then(setVoiceLanguages)
        .catch(() => setVoiceLanguages([]));
  }, [importMode]);
  useEffect(
    () => () => {
      recorder.current?.cancel();
      if (timeout.current) clearTimeout(timeout.current);
    },
    [],
  );
  async function run(work: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await work();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function attach(list: FileList | null) {
    if (!list) return;
    await run(async () => {
      let nextText = text;
      const nextImages = [...images];
      const names = [...files];
      for (const file of Array.from(list)) {
        const result = await readAttachment(file);
        nextText = [nextText, result.text].filter(Boolean).join("\n\n");
        nextImages.push(...result.images);
        names.push(file.name);
      }
      if (nextText.length > 16000 || nextImages.length > 4)
        throw new Error(
          "Use up to 16,000 characters or four images per import.",
        );
      setText(nextText);
      setImages(nextImages);
      setFiles(names);
      setProposal(null);
    });
    if (fileInput.current) fileInput.current.value = "";
  }
  async function stopVoice() {
    setRecording(false);
    if (timeout.current) clearTimeout(timeout.current);
    const active = recorder.current;
    recorder.current = null;
    if (!active) return;
    await run(async () => {
      const transcript = await coreApi.transcribe(
        await active.stop(),
        voiceLanguage,
      );
      setText((previous) => `${previous}${previous ? "\n" : ""}${transcript}`);
      setProposal(null);
    });
  }
  async function voice() {
    if (recording) {
      await stopVoice();
      return;
    }
    await run(async () => {
      const languages = voiceLanguages ?? (await coreApi.voiceLanguages());
      setVoiceLanguages(languages);
      if (
        !languages.some(
          (name) => voiceLanguage === "auto" || name.startsWith(voiceLanguage),
        )
      )
        throw new Error(
          "Enable the selected local speech language in your system settings, or paste a transcript.",
        );
      recorder.current = await recordVoice();
      setRecording(true);
      timeout.current = setTimeout(() => void stopVoice(), 60000);
    });
  }
  function clear() {
    setText("");
    setImages([]);
    setFiles([]);
    setProposal(null);
    setError("");
  }
  async function save(drafts: GoalDraft[], token: string) {
    await onSave(drafts, token);
    clear();
  }
  return (
    <section
      className="core-composer"
      aria-label={importMode ? "Import goals" : "Talk to sumi"}
    >
      <div className="composer-heading">
        <span className="buddy-dot">b.</span>
        <div>
          <h2>
            {importMode
              ? "Bring your intentions here"
              : "What would you like to move forward?"}
          </h2>
          <p>
            {importMode
              ? "Paste notes, upload a PDF or screenshot, or record a voice dump."
              : "Add a goal directly, or ask sumi to organize your thoughts."}
          </p>
        </div>
      </div>
      <label className="sr-only" htmlFor="core-message">
        Your message or goals
      </label>
      <textarea
        id="core-message"
        maxLength={16000}
        value={text}
        placeholder="A goal, a question, or a few thoughts…"
        disabled={busy || disabled || recording}
        onChange={(e) => {
          setText(e.target.value);
          setProposal(null);
        }}
      />
      {files.length > 0 && (
        <div className="attachment-list">
          {files.map((file, index) => (
            <span key={`${index}-${file}`}>{file}</span>
          ))}
          <button
            className="text-button"
            aria-label="Remove attachments"
            disabled={busy}
            onClick={() => {
              setImages([]);
              setFiles([]);
              setProposal(null);
            }}
          >
            <X size={14} />
          </button>
        </div>
      )}
      {!!images.length && (
        <div className="image-previews">
          {images.map((image, index) => (
            <img
              key={index}
              src={`data:${image.mime};base64,${image.data}`}
              alt={`Import attachment ${index + 1}`}
            />
          ))}
        </div>
      )}
      <div className="composer-actions">
        <input
          ref={fileInput}
          type="file"
          accept=".pdf,.txt,.md,.csv,image/png,image/jpeg,image/webp"
          multiple
          hidden
          onChange={(e) => void attach(e.target.files)}
        />
        <button
          className="text-button"
          disabled={busy || disabled || recording}
          onClick={() => fileInput.current?.click()}
        >
          <FileUp size={16} />
          Upload
        </button>
        <button
          className={`text-button ${recording ? "recording" : ""}`}
          disabled={busy || disabled || !desktop}
          onClick={() => void voice()}
        >
          {recording ? <Square size={15} /> : <Mic size={16} />}{" "}
          {recording ? "Stop recording" : "Voice"}
        </button>
        <span className="composer-spacer" />
        <button
          className="secondary"
          disabled={
            busy ||
            disabled ||
            !desktop ||
            !text.trim() ||
            !!images.length ||
            recording
          }
          onClick={() =>
            void run(() => save(textGoals(text), crypto.randomUUID()))
          }
        >
          <Plus size={15} />
          Add as goals
        </button>
        <button
          disabled={
            busy ||
            disabled ||
            !desktop ||
            (!text.trim() && !images.length) ||
            recording
          }
          onClick={() =>
            void run(async () => {
              const result = await coreApi.propose(text, images, true);
              setProposal(result);
              setBatch(crypto.randomUUID());
            })
          }
        >
          <ArrowUp size={15} />
          {busy ? "Working…" : "Ask sumi"}
        </button>
      </div>
      {importMode && (
        <label className="helper">
          Local voice language
          <select
            value={voiceLanguage}
            disabled={busy || disabled || recording}
            onChange={(e) => setVoiceLanguage(e.target.value)}
          >
            <option value="auto">
              {isMac ? "System language" : "Automatic / mixed"}
            </option>
            <option value="ru">Russian</option>
            <option value="en">English</option>
          </select>
          <span>
            Local speech languages:{" "}
            {voiceLanguages === null
              ? "checking…"
              : voiceLanguages.join(", ") || "none installed"}
            .{" "}
            {isMac
              ? "Recognition stays on this Mac; language availability depends on installed Dictation support."
              : "Mixed speech needs both Russian and English speech packs."}{" "}
            Review the transcript before asking sumi.
          </span>
        </label>
      )}
      <p className="helper">
        Ask sumi sends this draft to Claude when AI assistance is enabled in
        Settings. Review attachments before asking; suggestions need your
        acceptance.
        {mock ? " AI_MOCK is enabled: no provider request is sent." : ""}
      </p>
      {recording && (
        <p role="status">
          Recording for up to 60 seconds. Stop to create a local transcript.
        </p>
      )}
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {proposal && (
        <div className="import-proposal" aria-label="Review import proposal">
          <p className="buddy-reply">{proposal.reply}</p>
          {proposal.drafts.map((draft, index) => (
            <fieldset key={index} disabled={busy}>
              <legend>Proposed goal {index + 1}</legend>
              <label>
                Goal
                <input
                  maxLength={500}
                  value={draft.title}
                  onChange={(e) =>
                    setProposal({
                      ...proposal,
                      drafts: proposal.drafts.map((d, i) =>
                        i === index ? { ...d, title: e.target.value } : d,
                      ),
                    })
                  }
                />
              </label>
              <label>
                Area
                <input
                  maxLength={80}
                  value={draft.area}
                  onChange={(e) =>
                    setProposal({
                      ...proposal,
                      drafts: proposal.drafts.map((d, i) =>
                        i === index ? { ...d, area: e.target.value } : d,
                      ),
                    })
                  }
                />
              </label>
              <label>
                Steps, one per line
                <textarea
                  value={draft.steps.join("\n")}
                  onChange={(e) =>
                    setProposal({
                      ...proposal,
                      drafts: proposal.drafts.map((d, i) =>
                        i === index
                          ? { ...d, steps: e.target.value.split("\n") }
                          : d,
                      ),
                    })
                  }
                />
              </label>
              <button
                className="text-button"
                onClick={() =>
                  setProposal({
                    ...proposal,
                    drafts: proposal.drafts.filter((_, i) => i !== index),
                  })
                }
              >
                Remove from proposal
              </button>
            </fieldset>
          ))}
          <div className="settings-actions">
            {!!proposal.drafts.length && (
              <button
                disabled={
                  busy ||
                  proposal.drafts.some(
                    (d) =>
                      !d.title.trim() ||
                      d.steps.some((s) => !s.trim()) ||
                      d.steps.length > 20,
                  )
                }
                onClick={() => void run(() => save(proposal.drafts, batch))}
              >
                Confirm {proposal.drafts.length}{" "}
                {proposal.drafts.length === 1 ? "goal" : "goals"}
              </button>
            )}
            <button
              className="text-button"
              disabled={busy}
              onClick={() => setProposal(null)}
            >
              Discard proposal
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
