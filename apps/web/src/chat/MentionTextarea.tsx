import {
  useEffect,
  useId,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type Ref,
} from "react";
import { autoUpdate, flip, FloatingPortal, offset, shift, useFloating } from "@floating-ui/react";
import { EditorState, TextSelection } from "prosemirror-state";
import { Decoration, DecorationSet, EditorView } from "prosemirror-view";
import { history, undo, redo } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import { baseKeymap } from "prosemirror-commands";
import { Bold, Italic } from "lucide-react";
import { emojiSegments } from "./emoji.ts";
import { emojiName } from "./reactors.ts";
import { draftMentionSegments, type MentionCandidate } from "./mentions.ts";
import { linkSegments } from "./links.ts";
import {
  composerDocument,
  composerSource,
  displayedOffset,
  formatComposer,
  normalizeComposer,
  sourceOffset,
} from "./rich-text.ts";

/** Source offsets let emoji/mention insertion keep using the stored draft format. */
export interface ComposerInput extends HTMLDivElement {
  readonly value: string;
  readonly selectionStart: number;
  readonly selectionEnd: number;
  readonly disabled: boolean;
  setSelectionRange(start: number, end: number): void;
  replaceText(start: number, end: number, text: string): boolean;
}

type Props = Omit<HTMLAttributes<HTMLDivElement>, "onChange" | "onSelect"> & {
  ref?: Ref<ComposerInput>;
  value: string;
  disabled?: boolean;
  readOnly?: boolean;
  rows?: number;
  placeholder?: string;
  people?: MentionCandidate[];
  specialMentions?: boolean;
  onValueChange: (value: string, input: ComposerInput) => void;
  onSelect?: (event: { currentTarget: ComposerInput }) => void;
};

/** A real rich-text editing surface; Markdown exists only at the draft boundary. */
export default function MentionTextarea({
  ref,
  value,
  disabled,
  readOnly,
  rows = 1,
  placeholder,
  people,
  specialMentions = false,
  onValueChange,
  onSelect,
  ...props
}: Props) {
  const input = useRef<ComposerInput>(null);
  const view = useRef<EditorView>(undefined);
  const current = useRef({ value, disabled, readOnly, people, specialMentions, onValueChange, onSelect });
  current.current = { value, disabled, readOnly, people, specialMentions, onValueChange, onSelect };
  const source = useRef(value);
  useImperativeHandle(ref, () => input.current!, []);
  const [selection, setSelection] = useState(false);
  const [, renderSelection] = useState(0);
  const formatting = useFloating({
    open: selection && !disabled && !readOnly,
    placement: "top",
    strategy: "fixed",
    middleware: [offset(8), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const trackSelection = () => {
    const editor = view.current;
    if (!editor || editor.composing) return;
    setSelection(!editor.state.selection.empty && editor.hasFocus());
    renderSelection((revision) => revision + 1);
    current.current.onSelect?.({ currentTarget: input.current! });
    formatting.refs.setPositionReference({
      contextElement: input.current!,
      getBoundingClientRect: () => {
        const { from, to } = editor.state.selection;
        const start = editor.coordsAtPos(from);
        const end = editor.coordsAtPos(to);
        const bounds = editor.dom.getBoundingClientRect();
        return new DOMRect(
          Math.max(bounds.left, Math.min(start.left, bounds.right)),
          Math.max(bounds.top, start.top),
          Math.max(0, end.right - start.left),
          start.bottom - start.top,
        );
      },
    });
  };
  const track = useRef(trackSelection);
  track.current = trackSelection;
  const format = (style: "bold" | "italic") => {
    const editor = view.current;
    if (!editor || editor.composing || disabled || readOnly) return;
    editor.focus();
    formatComposer(style)(editor.state, editor.dispatch);
    track.current();
  };

  const [name, setName] = useState<string>();
  const tooltipId = useId();
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const generation = useRef(0);
  const emojiTarget = useRef<HTMLElement | null>(null);
  const press = useRef<{ x: number; y: number } | undefined>(undefined);
  const close = () => {
    clearTimeout(timer.current);
    emojiTarget.current = null;
    generation.current++;
    setName(undefined);
  };
  const tooltip = useFloating({
    open: !!name,
    placement: "top",
    strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const inspect = (target: HTMLElement, delay: number) => {
    const emoji = target.closest<HTMLElement>("[data-emoji]");
    if (emoji === emojiTarget.current) return;
    close();
    if (!emoji) return;
    emojiTarget.current = emoji;
    const request = generation.current;
    timer.current = setTimeout(() => {
      void emojiName(emoji.textContent ?? "").then((label) => {
        if (generation.current !== request) return;
        tooltip.refs.setPositionReference(emoji);
        setName(label);
      });
    }, delay);
  };
  useEffect(() => {
    close();
    return () => {
      clearTimeout(timer.current);
      generation.current++;
    };
  }, [value]);

  useLayoutEffect(() => {
    const field = input.current!;
    const plugins = () => [
      history(),
      keymap({
        "Mod-z": undo,
        "Shift-Mod-z": redo,
        "Mod-y": redo,
        Enter: (state, dispatch) => {
          dispatch?.(state.tr.insertText("\n"));
          return true;
        },
        "Shift-Enter": (state, dispatch) => {
          dispatch?.(state.tr.insertText("\n"));
          return true;
        },
      }),
      keymap(baseKeymap),
    ];
    const editor = new EditorView(
      { mount: field },
      {
        state: EditorState.create({ doc: composerDocument(source.current), plugins: plugins() }),
        editable: () => !current.current.disabled && !current.current.readOnly,
        dispatchTransaction(transaction) {
          if (transaction.docChanged && !editor.composing) normalizeComposer(transaction);
          editor.updateState(editor.state.apply(transaction));
          if (transaction.docChanged) {
            source.current = composerSource(editor.state.doc);
            current.current.onValueChange(source.current, field);
          }
          track.current();
        },
        decorations(state) {
          const decorations: Decoration[] = [];
          let position = 0;
          for (const segment of draftMentionSegments(
            state.doc.textContent,
            current.current.people,
            current.current.specialMentions,
          )) {
            if (segment.mention)
              decorations.push(
                Decoration.inline(position, position + segment.text.length, { class: "chat-draft-mention" }),
              );
            for (const part of linkSegments(segment.text)) {
              if (part.href) {
                decorations.push(
                  Decoration.inline(position, position + part.text.length, {
                    class: "chat-draft-link",
                    spellcheck: "false",
                  }),
                  // Reserve decorative icon space on the first character, without
                  // adding a document position, clipboard character or keyboard stop.
                  Decoration.inline(position, position + Array.from(part.text)[0].length, { class: "chat-link-start" }),
                );
              }
              position += part.text.length;
            }
          }
          position = 0;
          for (const part of emojiSegments(state.doc.textContent)) {
            if (part.emoji)
              decorations.push(Decoration.inline(position, position + part.text.length, { "data-emoji": "" }));
            position += part.text.length;
          }
          return DecorationSet.create(state.doc, decorations);
        },
        handlePaste(editor, event) {
          // Never import arbitrary HTML, remote images or a pasted link's hidden href.
          // Native file attachment handling runs in the React capture handler first.
          const text = event.clipboardData?.getData("text/plain");
          if (text === undefined) return false;
          editor.dispatch(editor.state.tr.insertText(text.replace(/\r\n?/g, "\n")).setMeta("paste", true));
          return true;
        },
        handleDrop(editor, event) {
          if (event.dataTransfer?.files.length) return true;
          const text = event.dataTransfer?.getData("text/plain");
          if (!text) return true;
          const position = editor.posAtCoords({ left: event.clientX, top: event.clientY });
          if (position) editor.dispatch(editor.state.tr.insertText(text, position.pos));
          return true;
        },
      },
    );
    view.current = editor;
    Object.defineProperties(field, {
      value: { configurable: true, get: () => source.current },
      disabled: { configurable: true, get: () => !!current.current.disabled },
      // React restores focus using these native-style setters when an overlay closes.
      selectionStart: {
        configurable: true,
        get: () => sourceOffset(source.current, editor.state.selection.from),
        set: (start: number) => field.setSelectionRange(start, Math.max(start, field.selectionEnd)),
      },
      selectionEnd: {
        configurable: true,
        get: () => sourceOffset(source.current, editor.state.selection.to, !editor.state.selection.empty),
        set: (end: number) => field.setSelectionRange(Math.min(field.selectionStart, end), end),
      },
    });
    field.setSelectionRange = (start, end) => {
      editor.dispatch(
        editor.state.tr.setSelection(
          TextSelection.create(
            editor.state.doc,
            displayedOffset(source.current, start),
            displayedOffset(source.current, end),
          ),
        ),
      );
    };
    field.replaceText = (start, end, text) => {
      if (!editor.editable || editor.composing) return false;
      const change = normalizeComposer(
        editor.state.tr
          .setSelection(
            TextSelection.create(
              editor.state.doc,
              displayedOffset(source.current, start),
              displayedOffset(source.current, end),
            ),
          )
          .insertText(text),
      );
      if (Array.from(composerSource(change.doc)).length > 4000) return false;
      editor.dispatch(change);
      editor.focus();
      return true;
    };
    return () => {
      editor.destroy();
      view.current = undefined;
    };
  }, []);

  useLayoutEffect(() => {
    const editor = view.current!;
    if (value !== source.current && !editor.composing) {
      const doc = composerDocument(value);
      source.current = value;
      if (!value) {
        // A sent/discarded draft must not be resurrected by undo in the next message.
        editor.updateState(EditorState.create({ doc, plugins: editor.state.plugins }));
      } else {
        editor.dispatch(editor.state.tr.replaceWith(0, editor.state.doc.content.size, doc.content));
      }
    }
    editor.setProps({ editable: () => !disabled && !readOnly });
  }, [value, disabled, readOnly, people, specialMentions]);

  return (
    <div className="chat-mention-input" data-disabled={disabled || undefined}>
      <div
        {...props}
        ref={input}
        className={`chat-rich-input ${props.className ?? ""}`}
        role="textbox"
        aria-multiline="true"
        aria-disabled={disabled || undefined}
        aria-readonly={readOnly || undefined}
        aria-label={props["aria-label"] ?? placeholder ?? "Message"}
        aria-keyshortcuts="Control+B Meta+B Control+I Meta+I"
        aria-description={
          props["aria-description"] ??
          "Select text to format it. Control or Command B for bold, I for italic. Formatting cannot split a link."
        }
        aria-describedby={
          name ? [props["aria-describedby"], tooltipId].filter(Boolean).join(" ") : props["aria-describedby"]
        }
        data-placeholder={placeholder}
        data-empty={!value || undefined}
        style={{ ...props.style, minHeight: rows > 1 ? `${rows * 1.4 + 1.25}em` : undefined }}
        tabIndex={disabled ? -1 : 0}
        onKeyDownCapture={(event) => {
          close();
          if (!event.nativeEvent.isComposing && !event.altKey && (event.metaKey || event.ctrlKey)) {
            const key = event.key.toLowerCase();
            if (key === "b" || key === "i") {
              event.preventDefault();
              format(key === "b" ? "bold" : "italic");
              return;
            }
          }
          if (selection && event.key === "Escape") {
            event.preventDefault();
            setSelection(false);
            return;
          }
          props.onKeyDown?.(event);
        }}
        onKeyDown={undefined}
        onPasteCapture={props.onPaste}
        onPaste={undefined}
        onCompositionStart={(event) => {
          setSelection(false);
          props.onCompositionStart?.(event);
        }}
        onCompositionEnd={(event) => {
          props.onCompositionEnd?.(event);
          requestAnimationFrame(() => {
            const editor = view.current;
            if (!editor || editor.composing) return;
            const transaction = normalizeComposer(editor.state.tr);
            if (transaction.docChanged) editor.dispatch(transaction);
            track.current();
          });
        }}
        onBlur={(event) => {
          close();
          if (!formatting.refs.floating.current?.contains(event.relatedTarget)) setSelection(false);
          props.onBlur?.(event);
        }}
        onPointerMove={(event) => {
          if (event.pointerType === "mouse" && !event.buttons) inspect(event.target as HTMLElement, 400);
          else if (press.current && Math.hypot(event.clientX - press.current.x, event.clientY - press.current.y) > 10)
            close();
          props.onPointerMove?.(event);
        }}
        onPointerDown={(event) => {
          close();
          press.current = undefined;
          if (event.pointerType !== "mouse" && event.isPrimary) {
            press.current = { x: event.clientX, y: event.clientY };
            inspect(event.target as HTMLElement, 500);
          }
          props.onPointerDown?.(event);
        }}
        onPointerUp={(event) => {
          if (press.current) {
            clearTimeout(timer.current);
            if (name) timer.current = setTimeout(close, 1500);
            else close();
            press.current = undefined;
          }
          props.onPointerUp?.(event);
        }}
        onPointerCancel={(event) => {
          close();
          press.current = undefined;
          props.onPointerCancel?.(event);
        }}
        onPointerLeave={(event) => {
          close();
          props.onPointerLeave?.(event);
        }}
        onScroll={(event) => {
          close();
          formatting.update();
          props.onScroll?.(event);
        }}
      />
      {selection && !disabled && !readOnly && (
        <FloatingPortal root={input.current?.closest<HTMLElement>("dialog, [role='dialog'], .live-scene") ?? undefined}>
          <div
            ref={formatting.refs.setFloating}
            style={formatting.floatingStyles}
            className="chat-format-toolbar"
            role="group"
            aria-label="Message formatting"
            onPointerDown={(event) => event.preventDefault()}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget) && event.relatedTarget !== input.current)
                setSelection(false);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                input.current?.focus();
                setSelection(false);
              }
            }}
          >
            {(
              [
                ["bold", "Bold", Bold],
                ["italic", "Italic", Italic],
              ] as const
            ).map(([style, label, Icon]) => {
              const state = view.current!.state;
              const allowed = formatComposer(style)(state);
              return (
                <button
                  key={style}
                  type="button"
                  aria-label={label}
                  aria-pressed={state.doc.rangeHasMark(
                    state.selection.from,
                    state.selection.to,
                    state.schema.marks[style],
                  )}
                  title={
                    allowed
                      ? `${label} (${navigator.platform.includes("Mac") ? "⌘" : "Ctrl+"}${label[0]})`
                      : "This selection cannot be formatted without changing its text or link"
                  }
                  disabled={!allowed}
                  onClick={() => format(style)}
                >
                  <Icon size={17} aria-hidden="true" />
                </button>
              );
            })}
          </div>
        </FloatingPortal>
      )}
      {name && (
        <FloatingPortal root={input.current?.closest<HTMLElement>(".live-scene") ?? undefined}>
          <div
            ref={tooltip.refs.setFloating}
            style={tooltip.floatingStyles}
            className="chat-emoji-tooltip"
            role="tooltip"
            id={tooltipId}
          >
            :{name}:
          </div>
        </FloatingPortal>
      )}
    </div>
  );
}
