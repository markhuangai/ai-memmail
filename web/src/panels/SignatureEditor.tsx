import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  AlignCenter,
  AlignLeft,
  AlignRight,
  Bold,
  Image as ImageIcon,
  Italic,
  Link2,
  Redo2,
  Underline as UnderlineIcon,
  Undo2
} from "lucide-react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Link from "@tiptap/extension-link";
import Underline from "@tiptap/extension-underline";
import Image from "@tiptap/extension-image";
import TextAlign from "@tiptap/extension-text-align";
import { Color, FontFamily, FontSize, TextStyle } from "@tiptap/extension-text-style";
import {
  DEFAULT_HTML_SIGNATURE,
  DEFAULT_PLAIN_SIGNATURE,
  signaturePreviewHtml
} from "../configModel";
import type { EmailSignatureConfig, EmailSignatureFormat, MailboxConfig } from "../types";

type SignatureMode = EmailSignatureFormat | "notice";

const editorExtensions = [
  StarterKit.configure({
    blockquote: false,
    bulletList: false,
    code: false,
    codeBlock: false,
    heading: false,
    horizontalRule: false,
    link: false,
    listItem: false,
    orderedList: false,
    strike: false,
    underline: false
  }),
  Underline,
  Link.configure({
    autolink: false,
    linkOnPaste: true,
    openOnClick: false
  }),
  TextStyle,
  Color.configure({ types: ["textStyle"] }),
  FontFamily.configure({ types: ["textStyle"] }),
  FontSize.configure({ types: ["textStyle"] }),
  TextAlign.configure({ types: ["paragraph"] }),
  Image.configure({
    allowBase64: false,
    inline: true,
    resize: { enabled: true, minWidth: 24, alwaysPreserveAspectRatio: true }
  })
];

const fontOptions = [
  ["", "Default"],
  ["Arial, Helvetica, sans-serif", "Arial"],
  ["Georgia, serif", "Georgia"],
  ["Tahoma, Geneva, sans-serif", "Tahoma"],
  ["Verdana, Geneva, sans-serif", "Verdana"]
] as const;

const sizeOptions = [
  ["", "Default"],
  ["12px", "12"],
  ["14px", "14"],
  ["16px", "16"],
  ["18px", "18"],
  ["20px", "20"]
] as const;

export function SignatureEditor({
  mailbox,
  onChange
}: {
  mailbox: MailboxConfig;
  onChange: (signature: EmailSignatureConfig | null) => void;
}) {
  const signature = mailbox.signature ?? null;
  const mode: SignatureMode = signature?.format ?? "notice";
  const previewHtml = useMemo(() => signaturePreviewHtml(signature), [signature]);

  function setMode(nextMode: SignatureMode) {
    if (nextMode === "notice") {
      onChange(null);
      return;
    }
    if (nextMode === "plain_text") {
      onChange({
        format: "plain_text",
        content:
          signature?.format === "plain_text" && signature.content
            ? signature.content
            : DEFAULT_PLAIN_SIGNATURE
      });
      return;
    }
    onChange({
      format: "html",
      content:
        signature?.format === "html" && signature.content
          ? signature.content
          : DEFAULT_HTML_SIGNATURE
    });
  }

  function setPlainContent(content: string) {
    onChange({ format: "plain_text", content });
  }

  return (
    <fieldset className="checkbox-panel signature-panel">
      <legend>Email signature</legend>
      <div className="segmented-control" role="group" aria-label="Signature mode">
        <ModeButton active={mode === "notice"} label="Current notice" onClick={() => setMode("notice")} />
        <ModeButton active={mode === "plain_text"} label="Plain" onClick={() => setMode("plain_text")} />
        <ModeButton active={mode === "html"} label="HTML" onClick={() => setMode("html")} />
      </div>

      {mode === "plain_text" ? (
        <label>
          Plain text
          <textarea
            value={signature?.format === "plain_text" ? signature.content : ""}
            onChange={(event) => setPlainContent(event.target.value)}
          />
        </label>
      ) : null}

      {mode === "html" ? (
        <HtmlSignatureEditor
          content={signature?.format === "html" ? signature.content : DEFAULT_HTML_SIGNATURE}
          onChange={(content) => onChange({ format: "html", content })}
        />
      ) : null}

      <div className="signature-preview">
        <span>Preview</span>
        <iframe
          referrerPolicy="no-referrer"
          sandbox=""
          srcDoc={previewHtml}
          title={`Signature preview for ${mailbox.id}`}
        />
      </div>
    </fieldset>
  );
}

function ModeButton({
  active,
  label,
  onClick
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button className={active ? "active" : ""} onClick={onClick} type="button">
      {label}
    </button>
  );
}

function HtmlSignatureEditor({
  content,
  onChange
}: {
  content: string;
  onChange: (content: string) => void;
}) {
  const [linkUrl, setLinkUrl] = useState("");
  const [imageUrl, setImageUrl] = useState("");
  const [imageAlt, setImageAlt] = useState("");
  const [error, setError] = useState("");
  const editor = useEditor({
    extensions: editorExtensions,
    content,
    immediatelyRender: false,
    editorProps: {
      attributes: {
        "aria-label": "HTML signature editor",
        class: "signature-editor-surface"
      }
    },
    onUpdate({ editor: updatedEditor }) {
      onChange(updatedEditor.getHTML());
    }
  });

  useEffect(() => {
    if (!editor || editor.getHTML() === content) {
      return;
    }
    editor.commands.setContent(content, { emitUpdate: false });
  }, [content, editor]);

  function setLink() {
    if (!editor) {
      return;
    }
    const trimmed = linkUrl.trim();
    if (!trimmed) {
      editor.chain().focus().unsetLink().run();
      setError("");
      return;
    }
    if (!allowedLinkUrl(trimmed)) {
      setError("Link URL must start with http, https, mailto, or tel.");
      return;
    }
    editor.chain().focus().extendMarkRange("link").setLink({ href: trimmed }).run();
    setError("");
  }

  function insertImage() {
    if (!editor) {
      return;
    }
    const trimmedUrl = imageUrl.trim();
    const trimmedAlt = imageAlt.trim();
    if (!allowedImageUrl(trimmedUrl)) {
      setError("Image URL must start with https.");
      return;
    }
    if (!trimmedAlt) {
      setError("Image alt text is required.");
      return;
    }
    editor.chain().focus().setImage({ src: trimmedUrl, alt: trimmedAlt, width: 160 }).run();
    setError("");
  }

  return (
    <div className="rich-signature-editor">
      <div className="signature-toolbar" aria-label="HTML signature toolbar" role="toolbar">
        <IconButton active={editor?.isActive("bold") ?? false} label="Bold" onClick={() => editor?.chain().focus().toggleBold().run()}>
          <Bold aria-hidden="true" />
        </IconButton>
        <IconButton active={editor?.isActive("italic") ?? false} label="Italic" onClick={() => editor?.chain().focus().toggleItalic().run()}>
          <Italic aria-hidden="true" />
        </IconButton>
        <IconButton active={editor?.isActive("underline") ?? false} label="Underline" onClick={() => editor?.chain().focus().toggleUnderline().run()}>
          <UnderlineIcon aria-hidden="true" />
        </IconButton>
        <IconButton label="Undo" onClick={() => editor?.chain().focus().undo().run()}>
          <Undo2 aria-hidden="true" />
        </IconButton>
        <IconButton label="Redo" onClick={() => editor?.chain().focus().redo().run()}>
          <Redo2 aria-hidden="true" />
        </IconButton>
        <IconButton active={editor?.isActive({ textAlign: "left" }) ?? false} label="Align left" onClick={() => editor?.chain().focus().setTextAlign("left").run()}>
          <AlignLeft aria-hidden="true" />
        </IconButton>
        <IconButton active={editor?.isActive({ textAlign: "center" }) ?? false} label="Align center" onClick={() => editor?.chain().focus().setTextAlign("center").run()}>
          <AlignCenter aria-hidden="true" />
        </IconButton>
        <IconButton active={editor?.isActive({ textAlign: "right" }) ?? false} label="Align right" onClick={() => editor?.chain().focus().setTextAlign("right").run()}>
          <AlignRight aria-hidden="true" />
        </IconButton>
        <label className="signature-select">
          Font
          <select
            onChange={(event) =>
              event.target.value
                ? editor?.chain().focus().setFontFamily(event.target.value).run()
                : editor?.chain().focus().unsetFontFamily().run()
            }
          >
            {fontOptions.map(([value, label]) => (
              <option key={label} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label className="signature-select">
          Size
          <select
            onChange={(event) =>
              event.target.value
                ? editor?.chain().focus().setFontSize(event.target.value).run()
                : editor?.chain().focus().unsetFontSize().run()
            }
          >
            {sizeOptions.map(([value, label]) => (
              <option key={label} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label className="signature-color">
          Color
          <input
            aria-label="Text color"
            type="color"
            onChange={(event) => editor?.chain().focus().setColor(event.target.value).run()}
          />
        </label>
      </div>

      <div className="signature-asset-row">
        <label>
          Link URL
          <input value={linkUrl} onChange={(event) => setLinkUrl(event.target.value)} />
        </label>
        <button onClick={setLink} type="button">
          <Link2 aria-hidden="true" />
          Link
        </button>
        <label>
          Image URL
          <input value={imageUrl} onChange={(event) => setImageUrl(event.target.value)} />
        </label>
        <label>
          Image alt
          <input value={imageAlt} onChange={(event) => setImageAlt(event.target.value)} />
        </label>
        <button onClick={insertImage} type="button">
          <ImageIcon aria-hidden="true" />
          Image
        </button>
      </div>
      {error ? <p className="signature-error" role="alert">{error}</p> : null}
      <EditorContent editor={editor} />
    </div>
  );
}

function IconButton({
  active,
  children,
  label,
  onClick
}: {
  active?: boolean;
  children: ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className={active ? "active" : ""}
      onClick={onClick}
      title={label}
      type="button"
    >
      {children}
    </button>
  );
}

function allowedLinkUrl(value: string): boolean {
  return allowedProtocol(value, ["http:", "https:", "mailto:", "tel:"]);
}

function allowedImageUrl(value: string): boolean {
  return allowedProtocol(value, ["https:"]);
}

function allowedProtocol(value: string, protocols: string[]): boolean {
  try {
    return protocols.includes(new URL(value).protocol);
  } catch {
    return false;
  }
}
