const WINDOWS_RESERVED_BASENAME = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i;
const MAX_EXPORT_FILENAME_LENGTH = 180;

function truncateUtf8(value: string, maxBytes: number): string {
  let usedBytes = 0;
  let result = "";
  for (const character of value) {
    const characterBytes = new TextEncoder().encode(character).length;
    if (usedBytes + characterBytes > Math.max(1, maxBytes)) break;
    result += character;
    usedBytes += characterBytes;
  }
  return result;
}

export function buildExportFileName(title: string, extension: string): string {
  const safeExtension =
    extension.replace(/^\.+/, "").replace(/[^a-z0-9]/gi, "").toLowerCase() || "txt";
  let basename = title
    .replace(/[\u0000-\u001f\u007f<>:"/\\|?*]/g, " ")
    .replace(/\s+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^[._]+|[._]+$/g, "");

  if (!basename) {
    basename = "transcript";
  }
  if (WINDOWS_RESERVED_BASENAME.test(basename.split(".", 1)[0] ?? basename)) {
    basename = `_${basename}`;
  }

  const maxBasenameBytes = MAX_EXPORT_FILENAME_LENGTH - safeExtension.length - 1;
  basename = truncateUtf8(basename, maxBasenameBytes).replace(/[._]+$/g, "");
  if (!basename) {
    basename = "transcript";
  }

  return `${basename}.${safeExtension}`;
}
