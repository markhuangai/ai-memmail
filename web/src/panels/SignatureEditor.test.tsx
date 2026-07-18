import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, describe, expect, it } from "vitest";
import {
  DEFAULT_HTML_SIGNATURE,
  DEFAULT_PLAIN_SIGNATURE
} from "../configModel";
import { sampleConfig } from "../fixtures";
import type { EmailSignatureConfig, MailboxConfig } from "../types";
import { SignatureEditor } from "./SignatureEditor";

const jsdomRect: DOMRect = {
  bottom: 0,
  height: 0,
  left: 0,
  right: 0,
  top: 0,
  width: 0,
  x: 0,
  y: 0,
  toJSON: () => ({})
};

const jsdomRectList: DOMRectList = {
  0: jsdomRect,
  length: 1,
  item: (index: number) => (index === 0 ? jsdomRect : null),
  [Symbol.iterator]: () => [jsdomRect][Symbol.iterator]()
};

beforeAll(() => {
  Object.defineProperty(Range.prototype, "getBoundingClientRect", {
    configurable: true,
    value: () => jsdomRect
  });
  Object.defineProperty(Range.prototype, "getClientRects", {
    configurable: true,
    value: () => jsdomRectList
  });
});

function renderControlled(initialSignature: EmailSignatureConfig | null) {
  const changes: Array<EmailSignatureConfig | null> = [];

  function Harness() {
    const [signature, setSignature] = useState<EmailSignatureConfig | null>(
      initialSignature
    );
    const mailbox: MailboxConfig = {
      ...sampleConfig.mailboxes[0],
      id: "support",
      signature
    };

    return (
      <>
        <SignatureEditor
          mailbox={mailbox}
          onChange={(nextSignature) => {
            changes.push(nextSignature);
            setSignature(nextSignature);
          }}
        />
        <button
          onClick={() =>
            setSignature({ format: "html", content: "<p>Loaded</p>" })
          }
          type="button"
        >
          Load alternate HTML
        </button>
      </>
    );
  }

  return { changes, ...render(<Harness />) };
}

function previewSource(): string {
  const preview = screen.getByTitle("Signature preview for support");
  return preview.getAttribute("srcdoc") ?? "";
}

describe("SignatureEditor", () => {
  it("switches between legacy notice, plain text, and default HTML modes", () => {
    const { changes } = renderControlled(null);

    expect(screen.getByTitle("Signature preview for support")).toHaveAttribute(
      "sandbox",
      ""
    );
    expect(previewSource()).toContain("This automated reply was sent on Mark");

    fireEvent.click(screen.getByRole("button", { name: "Plain" }));
    expect(changes.at(-1)).toEqual({
      format: "plain_text",
      content: DEFAULT_PLAIN_SIGNATURE
    });
    expect(screen.getByLabelText("Plain text")).toHaveValue(
      DEFAULT_PLAIN_SIGNATURE
    );

    fireEvent.change(screen.getByLabelText("Plain text"), {
      target: { value: "Thanks\nMark <mark@example.com>" }
    });
    expect(changes.at(-1)).toEqual({
      format: "plain_text",
      content: "Thanks\nMark <mark@example.com>"
    });
    expect(previewSource()).toContain("Mark &lt;mark@example.com&gt;");

    fireEvent.click(screen.getByRole("button", { name: "Current notice" }));
    expect(changes.at(-1)).toBeNull();
    expect(screen.queryByLabelText("Plain text")).not.toBeInTheDocument();
    expect(previewSource()).toContain("This automated reply was sent on Mark");

    fireEvent.click(screen.getByRole("button", { name: "HTML" }));
    expect(changes.at(-1)).toEqual({
      format: "html",
      content: DEFAULT_HTML_SIGNATURE
    });
    expect(
      screen.getByRole("toolbar", { name: "HTML signature toolbar" })
    ).toBeInTheDocument();
  });

  it("preserves existing signature content when reselecting the active mode", () => {
    const plain = renderControlled({
      format: "plain_text",
      content: "--\nExisting"
    });

    fireEvent.click(screen.getByRole("button", { name: "Plain" }));
    expect(plain.changes.at(-1)).toEqual({
      format: "plain_text",
      content: "--\nExisting"
    });
    plain.unmount();

    const html = renderControlled({
      format: "html",
      content: "<p>Existing</p>"
    });
    fireEvent.click(screen.getByRole("button", { name: "HTML" }));
    expect(html.changes.at(-1)).toEqual({
      format: "html",
      content: "<p>Existing</p>"
    });
  });

  it("applies toolbar actions and validates links and images", async () => {
    const { changes } = renderControlled({
      format: "html",
      content: "<p>Mark</p>"
    });

    expect(
      await screen.findByLabelText("HTML signature editor")
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Bold" }));
    fireEvent.click(screen.getByRole("button", { name: "Italic" }));
    fireEvent.click(screen.getByRole("button", { name: "Underline" }));
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    fireEvent.click(screen.getByRole("button", { name: "Redo" }));
    fireEvent.click(screen.getByRole("button", { name: "Align left" }));
    fireEvent.click(screen.getByRole("button", { name: "Align center" }));
    fireEvent.click(screen.getByRole("button", { name: "Align right" }));
    fireEvent.change(screen.getByLabelText("Font"), {
      target: { value: "Arial, Helvetica, sans-serif" }
    });
    fireEvent.change(screen.getByLabelText("Font"), {
      target: { value: "" }
    });
    fireEvent.change(screen.getByLabelText("Size"), {
      target: { value: "16px" }
    });
    fireEvent.change(screen.getByLabelText("Size"), {
      target: { value: "" }
    });
    fireEvent.change(screen.getByLabelText("Text color"), {
      target: { value: "#ff0000" }
    });

    fireEvent.change(screen.getByLabelText("Link URL"), {
      target: { value: "not a url" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Link" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Link URL must start with http, https, mailto, or tel."
    );

    fireEvent.change(screen.getByLabelText("Link URL"), {
      target: { value: "   " }
    });
    fireEvent.click(screen.getByRole("button", { name: "Link" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Link URL"), {
      target: { value: "mailto:mark@example.com" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Link" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Image URL"), {
      target: { value: "http://example.com/signature.png" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Image" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Image URL must start with https."
    );

    fireEvent.change(screen.getByLabelText("Image URL"), {
      target: { value: "https://example.com/signature.png" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Image" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Image alt text is required."
    );

    fireEvent.change(screen.getByLabelText("Image alt"), {
      target: { value: "Mark logo" }
    });
    fireEvent.click(screen.getByRole("button", { name: "Image" }));

    await waitFor(() =>
      expect(
        changes.some(
          (signature) =>
            signature?.format === "html" &&
            signature.content.includes("https://example.com/signature.png") &&
            signature.content.includes("Mark logo")
        )
      ).toBe(true)
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("syncs editor content when the HTML signature prop changes", async () => {
    renderControlled({
      format: "html",
      content: "<p>Original</p>"
    });

    const editor = await screen.findByLabelText("HTML signature editor");
    expect(editor).toHaveTextContent("Original");

    fireEvent.click(screen.getByRole("button", { name: "Load alternate HTML" }));

    await waitFor(() => expect(editor).toHaveTextContent("Loaded"));
  });
});
