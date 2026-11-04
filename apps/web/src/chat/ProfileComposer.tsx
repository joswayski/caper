import { useEffect, useId, useLayoutEffect, useState, useSyncExternalStore, type RefObject } from "react";
import { Send } from "lucide-react";
import AttachmentPicker from "./AttachmentPicker.tsx";
import { getAccount, getRememberedAccount } from "../account/client.ts";
import { createDirectConversation, directStatus, listDirectConversations } from "../spaces/client.ts";
import { directMessageErrors } from "../spaces/direct-errors.ts";
import { apiError, forgetAccountSession, sharedAccountSession } from "./client.ts";
import { DraftAttachments, type DraftAttachment, type DraftEditing } from "./Attachments.tsx";
import { useComposerSuggestions } from "./ComposerSuggestions.tsx";
import EmojiButton from "./EmojiButton.tsx";
import { useKlipy } from "./GifButton.tsx";
import { shareKlipy, type ChatGif } from "./klipy.ts";
import MentionTextarea, { type ComposerInput } from "./MentionTextarea.tsx";
import type { AttachmentEdit } from "./edit.ts";
import { editableKind, type CompressionSettings } from "./prepare.ts";
import { isChatMessage, type ChatAttachment } from "./types.ts";
import { AttachmentUpload, MAX_ATTACHMENTS, UploadError, uploadSettings, type UploadState } from "./uploads.ts";
import {
  getVoiceSendOnStop,
  subscribeVoiceSendOnStop,
  VOICE_LABEL,
  voiceFilename,
  type VoiceRecording,
} from "./voice.ts";
import { useVoiceRecorder } from "./VoiceRecorder.tsx";

interface ProfileFile extends Partial<UploadState> {
  key: string;
  file: File;
  /** Object URL for an image or video thumbnail; revoked when the file leaves. */
  localUrl?: string;
  upload?: AttachmentUpload;
  preparing?: Promise<ChatAttachment>;
  spoiler?: boolean;
  editable?: "image" | "video";
  /** Applied to the original before uploading. */
  edit?: AttachmentEdit;
  /** A render of an edited video for its tile; revoked when the file leaves. */
  thumbnailUrl?: string;
  /** A staged voice message: uploaded as recorded, played from `localUrl`. */
  voice?: VoiceRecording;
}
type FilesCommand = {
  clientMessageId: string;
  text: string;
  attachmentIds: string[];
  spoilerAttachmentIds: string[];
};
type MessageCommand = (FilesCommand | { clientMessageId: string; text: ""; gif: ChatGif; query: string }) & {
  replyToId?: string;
  threadRootId?: string;
};
export interface InboxReplyTarget {
  accountId: string;
  channelId: string;
  messageId: string;
  threadRootId?: string;
}
/** A voice message sends on its own: only its attachment, never the typed text. */
export function voiceCommand(attachmentId: string, clientMessageId: string = crypto.randomUUID()): FilesCommand {
  return { clientMessageId, text: "", attachmentIds: [attachmentId], spoilerAttachmentIds: [] };
}
/** A recorded voice message this card owns until the DM confirms it. */
interface ProfileVoice {
  recording: VoiceRecording;
  upload?: AttachmentUpload;
  channelId?: string;
  /** Like a draft's command: retries resend exactly this until settled. */
  command?: MessageCommand;
  error?: string;
}
interface ProfileDraft {
  text: string;
  files: ProfileFile[];
  channelId?: string;
  /** Once POST starts, retries must send this exact command until settled. */
  command?: MessageCommand;
}
// Like conversation drafts, these live only in this signed-in page. Closing a
// card never discards files or an uncertain send; signing out reloads the page.
const drafts = new Map<string, ProfileDraft>();
const voices = new Map<string, ProfileVoice>();
const pending = new Set<string>();
const emptyDraft: ProfileDraft = { text: "", files: [] };
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
const notify = () => {
  for (const listener of listeners) listener();
};
const resolving = new Map<string, ReturnType<typeof createDirectConversation>>();
function resolveConversation(personId: string, username: string) {
  const current = resolving.get(personId);
  if (current) return current;
  const request = (async () => {
    const existing = (await listDirectConversations()).conversations.find((item) => item.peer.id === personId);
    // Creating an existing DM also accepts its request; inspect it first.
    if (existing && directStatus(existing) === "incoming")
      throw new Error("Open the conversation and accept this message request before replying.");
    const conversation = existing ?? (await createDirectConversation(username));
    if (conversation.peer.id !== personId) throw new Error("This username has changed. Reopen the person’s profile.");
    if (conversation.blocked) throw new Error("Unblock this person before messaging them.");
    if (directStatus(conversation) === "incoming")
      throw new Error("Open the conversation and accept this message request before replying.");
    return conversation;
  })().finally(() => resolving.delete(personId));
  resolving.set(personId, request);
  return request;
}

/** POSTs a DM command and checks the confirmation. A definitive refusal is
 * marked `final`: that command can never have committed, so it may be replaced. */
async function postDirectMessage(
  channelId: string,
  account: { id: string },
  session: { token: string },
  command: MessageCommand,
) {
  const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages`, {
    method: "POST",
    credentials: "same-origin",
    headers: { "content-type": "application/json", "x-caper-chat-token": session.token },
    body: JSON.stringify("gif" in command ? { ...command, query: undefined } : command),
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) {
    const reason = await apiError(response, "Message could not be sent.");
    const code = reason.code;
    const final =
      [400, 404, 409, 413, 422].includes(response.status) ||
      (response.status === 403 && !!code && code in directMessageErrors);
    if (response.status === 401) forgetAccountSession(session.token);
    throw Object.assign(reason, { final });
  }
  const message: unknown = await response.json();
  if (
    !isChatMessage(message) ||
    message.channelId !== channelId ||
    message.clientMessageId !== command.clientMessageId ||
    message.author.id !== account.id ||
    message.threadRootId !== command.threadRootId ||
    message.replyTo?.id !== command.replyToId
  )
    throw new Error("The chat service returned an invalid confirmation.");
}
const isFinal = (reason: unknown) => !!(reason as { final?: unknown } | undefined)?.final;
const failure = (reason: unknown) =>
  reason instanceof TypeError
    ? "Couldn’t confirm delivery. Check your connection."
    : reason instanceof Error
      ? reason.message
      : "Couldn’t send this message.";

export default function ProfileComposer({
  personId,
  username,
  inputRef: input,
  onSent,
  onStateChange,
  replyTarget,
}: {
  personId: string;
  username: string;
  /** The card focuses this field when it opens. */
  inputRef: RefObject<ComposerInput | null>;
  onSent: () => void;
  onStateChange: (state: { picking: boolean; sending: boolean }) => void;
  /** Replies stay in the original conversation; never resolve or accept a DM. */
  replyTarget?: InboxReplyTarget;
}) {
  const draftKey = replyTarget
    ? `inbox:${replyTarget.accountId}:${replyTarget.channelId}:${replyTarget.messageId}`
    : personId;
  const inputId = useId();
  const assertAccount = () => {
    if (replyTarget && getRememberedAccount()?.id !== replyTarget.accountId)
      throw new Error("Your account changed. Reopen Notifications before replying.");
  };
  const conversation = async () => {
    if (!replyTarget) return resolveConversation(personId, username);
    const account = await getAccount();
    if (account?.id !== replyTarget.accountId)
      throw new Error("Your account changed. Reopen Notifications before replying.");
    return { id: replyTarget.channelId };
  };
  const targeted = (command: MessageCommand): MessageCommand => ({
    ...command,
    ...(replyTarget?.threadRootId
      ? { threadRootId: replyTarget.threadRootId }
      : replyTarget
        ? { replyToId: replyTarget.messageId }
        : {}),
  });
  const draft = useSyncExternalStore(
    subscribe,
    () => drafts.get(draftKey) ?? emptyDraft,
    () => emptyDraft,
  );
  const sending = useSyncExternalStore(
    subscribe,
    () => pending.has(draftKey),
    () => false,
  );
  const voice = useSyncExternalStore(
    subscribe,
    () => voices.get(draftKey),
    () => undefined,
  );
  const [error, setError] = useState<string>();
  const [status, setStatus] = useState<string>();
  const [settings, setSettings] = useState<CompressionSettings>();
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  const [emojiOpen, setEmojiOpen] = useState(false);
  const klipy = useKlipy();
  const [choosingFile, setChoosingFile] = useState(false);
  const locked = sending || !!draft.command;
  const saveVoice = (next: ProfileVoice | undefined) => {
    if (next) voices.set(draftKey, next);
    else voices.delete(draftKey);
    notify();
  };
  /** Uploads the recording to the DM as recorded and sends it alone; typed text
   * and files stay in the card. Without a recording, retries the kept one. */
  const sendVoice = async (recording?: VoiceRecording) => {
    if (pending.has(draftKey)) return;
    if (recording) {
      voices.get(draftKey)?.upload?.release(true);
      saveVoice({ recording });
    }
    if (!voices.get(draftKey)) return;
    pending.add(draftKey);
    notify();
    setError(undefined);
    try {
      const account = await getAccount();
      if (!account?.username || !account.displayName) throw new Error("Sign in to send a message.");
      const destination = await conversation();
      let entry = voices.get(draftKey);
      if (!entry) return;
      if (entry.channelId && entry.channelId !== destination.id)
        throw new Error("The conversation changed. Discard the voice message and record it again.");
      const upload = entry.upload ?? AttachmentUpload.voice(destination.id, entry.recording);
      entry = { ...entry, upload, channelId: destination.id, error: undefined };
      saveVoice(entry);
      const session = await sharedAccountSession(account.id, account.displayName);
      const command = entry.command ?? targeted(voiceCommand((await upload.ensure()).id));
      assertAccount();
      if (voices.get(draftKey)?.upload !== upload) return;
      entry = { ...entry, command };
      saveVoice(entry);
      try {
        await postDirectMessage(destination.id, account, session, command);
      } catch (reason) {
        if (isFinal(reason)) entry = { ...entry, command: undefined };
        saveVoice(entry);
        throw reason;
      }
      upload.release();
      saveVoice(undefined);
      assertAccount();
      onSent();
    } catch (reason) {
      const latest = voices.get(draftKey);
      const message = reason instanceof UploadError ? reason.message : failure(reason);
      if (latest) saveVoice({ ...latest, error: message });
      else setError(message);
    } finally {
      pending.delete(draftKey);
      notify();
    }
  };
  const sendOnStop = useSyncExternalStore(subscribeVoiceSendOnStop, getVoiceSendOnStop, () => false);
  const recorder = useVoiceRecorder({
    voice: {
      // By default a recording is staged with the card's text and files; the
      // preference sends it on its own as soon as recording stops.
      onRecorded: (recording) => (getVoiceSendOnStop() ? void sendVoice(recording) : stageVoice(recording)),
      sendOnStop,
      blockedReason: draft.files.some((item) => item.voice)
        ? "Remove the voice message to record another"
        : draft.files.length >= MAX_ATTACHMENTS
          ? `You can attach up to ${MAX_ATTACHMENTS} files.`
          : undefined,
      sending: sending && !!voice && !voice.error,
      error: voice?.error,
      onRetry: () => void sendVoice(),
      onDiscard: () => {
        const discarded = voices.get(draftKey);
        // An uncertain send may have arrived, so only an unsent upload is cancelled.
        discarded?.upload?.release(!discarded.command);
        saveVoice(undefined);
        restoreFocus();
      },
    },
    // One send at a time; a kept voice message waits for Retry or Discard.
    disabled: locked || !settings || !!voice,
    returnFocus: input,
  });

  const save = (next: ProfileDraft) => {
    if (next.text || next.files.length || next.command) drafts.set(draftKey, next);
    else drafts.delete(draftKey);
    notify();
  };
  const suggestions = useComposerSuggestions({
    id: inputId,
    draft: draft.text,
    input,
    specialMentions: false,
    onTooLong: () => setError("Messages must be 4,000 characters or fewer."),
  });
  useEffect(() => {
    let active = true;
    void uploadSettings().then((value) => {
      if (active) {
        setSettings(value);
        setSettingsLoaded(true);
      }
    });
    return () => {
      active = false;
    };
  }, []);
  useEffect(() => {
    // Recording keeps the card open like a picker: an outside press is not a dismissal.
    onStateChange({ picking: emojiOpen || choosingFile || recorder.active, sending });
  }, [emojiOpen, choosingFile, recorder.active, sending, onStateChange]);

  // Grow with the draft, like the channel composer, up to the CSS max-height.
  useLayoutEffect(() => {
    const element = input.current;
    if (!element) return;
    const resize = () => {
      element.style.height = "0px";
      element.style.height = `${element.scrollHeight + element.offsetHeight - element.clientHeight}px`;
    };
    resize();
    let width = element.clientWidth;
    const observer = new ResizeObserver(() => {
      if (element.clientWidth === width) return;
      width = element.clientWidth;
      resize();
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [draft.text]);

  const restoreFocus = () => requestAnimationFrame(() => input.current?.focus());
  const updateFile = (key: string, change: Partial<ProfileFile>) => {
    const latest = drafts.get(draftKey);
    if (!latest?.files.some((file) => file.key === key)) return;
    save({ ...latest, files: latest.files.map((file) => (file.key === key ? { ...file, ...change } : file)) });
  };
  const ensureFile = (key: string): Promise<ChatAttachment> => {
    const item = drafts.get(draftKey)?.files.find((file) => file.key === key);
    if (!item || !settings) return Promise.reject(new Error("File uploads are unavailable. Your draft is kept."));
    if (item.upload) return item.upload.ensure();
    if (item.preparing) return item.preparing;
    const preparing = (async () => {
      try {
        const destination = await conversation();
        assertAccount();
        const latest = drafts.get(draftKey);
        const file = latest?.files.find((file) => file.key === key);
        if (!latest || !file) throw new Error("Upload cancelled.");
        if (latest.channelId && latest.channelId !== destination.id)
          throw new Error("The conversation changed. Remove the files and try again.");
        save({ ...latest, channelId: destination.id });
        const upload = file.voice
          ? AttachmentUpload.voice(destination.id, file.voice, (state) => updateFile(key, state))
          : new AttachmentUpload(destination.id, file.file, settings, (state) => updateFile(key, state));
        updateFile(key, { upload });
        // Upload owns its errors; an aborted, superseded edit must not overwrite
        // the replacement upload's state through this resolution error handler.
        return file.edit ? upload.setEdit(file.edit) : upload.ensure();
      } catch (reason) {
        updateFile(key, { error: reason instanceof Error ? reason.message : "Failed to upload." });
        throw reason;
      } finally {
        updateFile(key, { preparing: undefined });
      }
    })();
    updateFile(key, { preparing, error: undefined });
    return preparing;
  };
  useEffect(() => {
    const refresh = () => {
      const latest = drafts.get(draftKey);
      if (document.visibilityState !== "visible" || pending.has(draftKey) || latest?.command || !settings) return;
      for (const file of latest?.files ?? [])
        if (file.error || file.upload?.stale) void ensureFile(file.key).catch(() => {});
    };
    refresh();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [draftKey, settings]);
  const addFiles = (files: File[]) => {
    if (locked || !settings || !files.length) return;
    const room = MAX_ATTACHMENTS - draft.files.length;
    setError(files.length > room ? `You can attach up to ${MAX_ATTACHMENTS} files.` : undefined);
    const added = files.slice(0, room).map((file) => ({
      key: crypto.randomUUID(),
      file,
      localUrl: /^(image|video)\//.test(file.type) ? URL.createObjectURL(file) : undefined,
    }));
    save({ ...draft, files: [...draft.files, ...added] });
    for (const item of added) {
      void ensureFile(item.key).catch(() => {});
      void editableKind(item.file).then((editable) => {
        const latest = drafts.get(draftKey);
        if (!editable || !latest?.files.some((file) => file.key === item.key)) return;
        save({ ...latest, files: latest.files.map((file) => (file.key === item.key ? { ...file, editable } : file)) });
      });
    }
    restoreFocus();
  };
  const stageVoice = (recording: VoiceRecording) => {
    const latest = drafts.get(draftKey) ?? emptyDraft;
    if (pending.has(draftKey) || latest.command || latest.files.some((item) => item.voice)) return;
    if (latest.files.length >= MAX_ATTACHMENTS) {
      setError(`You can attach up to ${MAX_ATTACHMENTS} files.`);
      return;
    }
    const item: ProfileFile = {
      key: crypto.randomUUID(),
      file: new File([recording.blob], voiceFilename(recording.contentType), { type: recording.contentType }),
      localUrl: URL.createObjectURL(recording.blob),
      voice: recording,
    };
    save({ ...latest, files: [...latest.files, item] });
    void ensureFile(item.key).catch(() => {});
  };
  const removeFile = (key: string) => {
    const removed = draft.files.find((item) => item.key === key);
    removed?.upload?.release(true);
    if (removed?.localUrl) URL.revokeObjectURL(removed.localUrl);
    if (removed?.thumbnailUrl) URL.revokeObjectURL(removed.thumbnailUrl);
    save({ ...draft, files: draft.files.filter((item) => item.key !== key) });
    restoreFocus();
  };
  // Edits replace the upload as well as the local preview.
  const editing: DraftEditing = {
    source: (key) => draft.files.find((item) => item.key === key)?.file,
    save: (key, edit, preview) => {
      const latest = drafts.get(draftKey);
      const item = latest?.files.find((file) => file.key === key);
      if (!latest || !item || latest.command || pending.has(draftKey)) return;
      if (item.localUrl) URL.revokeObjectURL(item.localUrl);
      if (item.thumbnailUrl) URL.revokeObjectURL(item.thumbnailUrl);
      const url = (blob: Blob | undefined) => blob && URL.createObjectURL(blob);
      const localUrl = !edit ? url(item.file) : item.editable === "image" ? url(preview) : undefined;
      const thumbnailUrl = edit && item.editable === "video" ? url(preview) : undefined;
      save({
        ...latest,
        files: latest.files.map((file) =>
          file.key === key
            ? { ...file, edit, localUrl, thumbnailUrl, attachment: undefined, progress: undefined }
            : file,
        ),
      });
      if (item.upload) void item.upload.setEdit(edit).catch(() => {});
      else void ensureFile(key).catch(() => {});
      restoreFocus();
    },
  };
  const chooseEmoji = (emoji: string) => {
    const element = input.current;
    const start = element?.selectionStart ?? draft.text.length;
    if (!element?.replaceText(start, element.selectionEnd, emoji)) {
      setError("Messages must be 4,000 characters or fewer.");
    }
  };
  /** Sends the draft, or `gif` on its own, keeping the draft. */
  const submit = async (gif?: { gif: ChatGif; query: string }) => {
    if (pending.has(draftKey)) return;
    if (gif && draft.command) {
      setError("Retry or edit the unsent message first.");
      return;
    }
    if (!gif && !draft.command && !draft.text.trim() && !draft.files.length) return;
    if (!gif && Array.from(draft.text).length > 4_000) {
      setError("Messages must be 4,000 characters or fewer.");
      return;
    }
    pending.add(draftKey);
    notify();
    setError(undefined);
    setStatus("Sending…");
    try {
      const account = await getAccount();
      if (!account?.username || !account.displayName) throw new Error("Sign in to send a message.");
      const destination = await conversation();
      if (draft.channelId && draft.channelId !== destination.id)
        throw new Error("The conversation changed. Remove the files and try again.");
      const session = await sharedAccountSession(account.id, account.displayName);
      // An uncertain POST must retry its exact asset IDs, even after expiry.
      if (!draft.command && !gif) await Promise.all(draft.files.map((item) => ensureFile(item.key)));
      const latest = drafts.get(draftKey) ?? draft;
      const command =
        draft.command ??
        targeted(
          gif
            ? { clientMessageId: crypto.randomUUID(), text: "" as const, ...gif }
            : {
                clientMessageId: crypto.randomUUID(),
                text: draft.text,
                attachmentIds: latest.files.map((item) => item.attachment!.id),
                spoilerAttachmentIds: latest.files.filter((item) => item.spoiler).map((item) => item.attachment!.id),
              },
        );
      assertAccount();
      save({ ...latest, channelId: destination.id, command });
      setStatus("Sending…");
      try {
        await postDirectMessage(destination.id, account, session, command);
      } catch (reason) {
        if (isFinal(reason)) save({ ...(drafts.get(draftKey) ?? latest), command: undefined });
        throw reason;
      }
      if ("gif" in command) {
        // A GIF goes on its own; the typed draft and its files stay.
        save({ ...(drafts.get(draftKey) ?? latest), command: undefined });
        if (klipy) shareKlipy(klipy, command.gif, command.query);
        setStatus(undefined);
        assertAccount();
        onSent();
        return;
      }
      for (const item of latest.files) {
        item.upload?.release();
        if (item.localUrl) URL.revokeObjectURL(item.localUrl);
        if (item.thumbnailUrl) URL.revokeObjectURL(item.thumbnailUrl);
      }
      save({ text: "", files: [] });
      setStatus(undefined);
      assertAccount();
      onSent();
    } catch (reason) {
      // Keep completed uploads and the idempotent command, not just the text.
      setStatus(undefined);
      setError(
        drafts.get(draftKey)?.files.some((item) => item.error)
          ? undefined // The upload tray owns this failure and its retry guidance.
          : failure(reason),
      );
    } finally {
      pending.delete(draftKey);
      notify();
    }
  };

  const files: DraftAttachment[] = draft.files.map((item) => ({
    key: item.key,
    name: item.voice ? VOICE_LABEL : (item.name ?? item.file.name),
    localUrl: item.localUrl,
    localKind: item.localUrl && !item.voice ? (item.file.type.startsWith("video/") ? "video" : "image") : undefined,
    voice: item.voice && { durationMs: item.voice.durationMs, waveform: item.voice.waveform },
    editable: item.editable,
    edit: item.edit,
    thumbnailUrl: item.thumbnailUrl,
    sourceSize: item.file.size,
    storedSize: item.storedSize,
    compressing: item.compressing,
    progress: item.progress ?? 0,
    attachment: item.attachment,
    error: item.error,
    spoiler: item.spoiler,
  }));
  const canSend = !sending && (!!draft.text.trim() || !!draft.files.length);

  return (
    <div
      className="chat-profile-composer"
      onDragOver={(event) => {
        if (event.dataTransfer.types.includes("Files")) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onDrop={(event) => {
        if (!event.dataTransfer.files.length) return;
        event.preventDefault();
        event.stopPropagation();
        addFiles([...event.dataTransfer.files]);
      }}
    >
      {draft.command && !sending ? (
        <p className="chat-send-status">
          <span className="chat-send-error" role="alert">
            {error ?? "Delivery isn’t confirmed."}
          </span>
          <button type="button" onClick={() => void submit()}>
            Retry
          </button>
          <button
            type="button"
            onClick={() => {
              save({ ...draft, command: undefined });
              setError("The earlier copy may have arrived. Check the conversation before sending again.");
              restoreFocus();
            }}
          >
            Edit
          </button>
        </p>
      ) : (
        error && (
          <p className="chat-inline-error" role="alert">
            {error}
          </p>
        )
      )}
      {recorder.notice}
      <DraftAttachments
        drafts={files}
        onRemove={locked ? undefined : removeFile}
        onRetry={locked ? undefined : (key) => void ensureFile(key).catch(() => {})}
        onToggleSpoiler={
          locked
            ? undefined
            : (key) =>
                save({
                  ...draft,
                  files: draft.files.map((item) => (item.key === key ? { ...item, spoiler: !item.spoiler } : item)),
                })
        }
        editing={editing}
      />
      <form
        data-attach
        data-voice
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <AttachmentPicker
          disabled={locked || !settings}
          count={draft.files.length}
          title={settings ? "Attach files" : settingsLoaded ? "File uploads are unavailable" : "Checking file uploads…"}
          onAdd={addFiles}
          onPickingChange={setChoosingFile}
          onRestoreFocus={() => input.current?.focus()}
        />
        <label className="sr-only" htmlFor={inputId}>
          {replyTarget ? "Reply to notification" : `Message @${username}`}
        </label>
        {suggestions.popup}
        <MentionTextarea
          id={inputId}
          ref={input}
          people={replyTarget ? [] : [{ id: personId, username, displayName: username }]}
          rows={1}
          value={draft.text}
          disabled={locked}
          placeholder={replyTarget ? "Write a reply…" : `Message @${username}`}
          enterKeyHint="send"
          aria-describedby={`${inputId}-hint`}
          {...suggestions.textarea}
          onValueChange={(value, input) => {
            save({ ...draft, text: value });
            suggestions.change(input);
            setError(undefined);
          }}
          onBlur={suggestions.blur}
          onPaste={(event) => {
            if (event.clipboardData.files.length && settings && !locked) {
              event.preventDefault();
              addFiles([...event.clipboardData.files]);
            }
          }}
          onKeyDown={(event) => {
            if (suggestions.keyDown(event)) {
              event.stopPropagation();
              return;
            }
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void submit();
            }
          }}
        />
        <span id={`${inputId}-hint`} className="sr-only">
          Enter to send. Shift+Enter for a new line. Your draft is kept if you close this card.
        </span>
        <div className="chat-composer-tools">
          <EmojiButton
            disabled={locked}
            onSelect={chooseEmoji}
            onSendGif={(gif, query) => void submit({ gif, query })}
            onOpenChange={setEmojiOpen}
            returnFocus={input}
          />
          {recorder.button}
          <button
            type="submit"
            className="chat-composer-send"
            aria-label={draft.command ? "Retry send" : replyTarget ? "Send reply" : `Send to @${username}`}
            title="Send"
            disabled={!canSend}
          >
            <Send size={16} aria-hidden="true" />
          </button>
        </div>
        {recorder.bar}
      </form>
      {status && (
        <p className="chat-profile-status" role="status">
          {status}
        </p>
      )}
    </div>
  );
}
