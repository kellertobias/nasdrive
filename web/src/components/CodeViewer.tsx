import CodeMirror from "@uiw/react-codemirror";
import { monokai } from "@uiw/codemirror-theme-monokai";
import { loadLanguage } from "@uiw/codemirror-extensions-langs";

/**
 * Syntax-highlighted read-only view of a text file.
 *
 * Split into its own module so CodeMirror and its language grammars stay out of
 * the entry bundle — they are only needed once someone opens a text preview,
 * which most file-browsing sessions never do.
 */
export function CodeViewer({ content, name }: { content: string; name: string }) {
  const ext = name.split(".").pop()?.toLowerCase();
  let langExtension;
  try {
    langExtension = ext
      ? loadLanguage(ext as Parameters<typeof loadLanguage>[0])
      : undefined;
  } catch {
    // Ignore unsupported languages
  }

  return (
    <CodeMirror
      value={content}
      editable={false}
      theme={monokai}
      extensions={langExtension ? [langExtension] : []}
      basicSetup={{
        lineNumbers: true,
        foldGutter: true,
        highlightActiveLine: true,
      }}
    />
  );
}
