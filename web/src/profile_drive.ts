// Google app-data wire checks shared with the native transport
// (docs/agents/PROFILE_DRIVE.md). Files are immutable operation records;
// anything unowned, changed or unparseable never becomes an empty profile.
import {
  MAX_RECORD_BYTES,
  PAGE_SIZE,
  isUuid,
  type Operation,
} from "./profile_types";

export interface DriveFile {
  id: string;
  name: string;
  mimeType: string;
  size: number;
  trashed: boolean;
  ownedByMe: boolean;
  appProperties: Record<string, string>;
  md5Checksum?: string;
}
export interface ProfileFileIdentity {
  operation: string;
  profile: string;
  generation: string;
  sha256: string;
}
export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}
export function fileName(operation: string) {
  return `shep-profile-${operation}.json`;
}
const HEX64 = /^[0-9a-f]{64}$/;
/// Parse one Drive file resource; unrelated app-data files return null and
/// malformed profile files throw so they stay visible as failures.
export function parseDriveFile(value: unknown): DriveFile | null {
  if (typeof value !== "object" || value === null) return null;
  const file = value as Record<string, unknown>;
  const properties = file.appProperties;
  if (
    typeof properties !== "object" ||
    properties === null ||
    (properties as Record<string, unknown>).shepType !== "profile"
  )
    return null;
  const id = file.id;
  if (typeof id !== "string" || !/^[A-Za-z0-9_-]{1,255}$/.test(id))
    throw new Error("A profile file has an invalid identity.");
  const size = Number(file.size);
  if (!Number.isInteger(size) || size < 0 || size > MAX_RECORD_BYTES)
    throw new Error("A profile file has an invalid or oversized declaration.");
  return {
    id,
    name: typeof file.name === "string" ? file.name : "",
    mimeType: typeof file.mimeType === "string" ? file.mimeType : "",
    size,
    trashed: file.trashed === true,
    ownedByMe: file.ownedByMe === true,
    appProperties: Object.fromEntries(
      Object.entries(properties as Record<string, unknown>).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    ),
    md5Checksum:
      typeof file.md5Checksum === "string" ? file.md5Checksum : undefined,
  };
}
/// Metadata checks before any media is read.
export function verifyFileMetadata(
  file: DriveFile,
  namespaceDigest: string,
): ProfileFileIdentity {
  const p = file.appProperties;
  if (file.trashed) throw new Error("This profile file was moved to the bin.");
  if (!file.ownedByMe)
    throw new Error("This profile file is not owned by the signed-in account.");
  if (file.mimeType !== "application/json")
    throw new Error("This profile file has an unexpected type.");
  if (p.shepFormat !== "operation-v1")
    throw new Error(
      "This profile uses an unsupported version or capability. Update Shep before syncing it.",
    );
  if (p.shepNamespace !== namespaceDigest)
    throw new Error(
      "This profile belongs to a different Shep application namespace. Check the configured namespace before syncing.",
    );
  if (
    !isUuid(p.shepProfile) ||
    !isUuid(p.shepGeneration) ||
    !isUuid(p.shepOperation) ||
    !HEX64.test(p.shepSha256 ?? "")
  )
    throw new Error("This profile file has invalid identity properties.");
  if (file.name !== fileName(p.shepOperation))
    throw new Error("This profile file name does not match its operation.");
  return {
    operation: p.shepOperation,
    profile: p.shepProfile,
    generation: p.shepGeneration,
    sha256: p.shepSha256,
  };
}
/// The downloaded media must be the exact declared bytes and decode to the
/// identity the metadata claims.
export async function verifyMedia(
  file: DriveFile,
  identity: ProfileFileIdentity,
  media: string,
  decoded: Operation,
): Promise<void> {
  const bytes = new TextEncoder().encode(media).length;
  if (bytes !== file.size)
    throw new Error("This profile file changed while it was being read.");
  if ((await sha256Hex(media)) !== identity.sha256)
    throw new Error("This profile file does not match its recorded digest.");
  if (
    decoded.operation !== identity.operation ||
    decoded.profile !== identity.profile ||
    decoded.generation !== identity.generation
  )
    throw new Error("This profile record does not match its file identity.");
}
export interface ListPage {
  files: DriveFile[];
  nextPageToken: string | null;
}
export function parseListPage(value: unknown): ListPage {
  if (typeof value !== "object" || value === null)
    throw new Error("Drive returned an invalid listing.");
  const page = value as Record<string, unknown>;
  if (page.incompleteSearch !== false || !Array.isArray(page.files))
    throw new Error(
      "Drive returned an incomplete listing. Retry discovery before trusting these profiles.",
    );
  if (page.files.length > PAGE_SIZE)
    throw new Error("Drive returned more files than one page allows.");
  const files: DriveFile[] = [];
  const seen = new Set<string>();
  for (const raw of page.files) {
    const file = parseDriveFile(raw);
    if (!file) continue;
    if (seen.has(file.id))
      throw new Error("Drive listed the same file twice in one page.");
    seen.add(file.id);
    files.push(file);
  }
  const token = page.nextPageToken;
  if (token !== undefined && (typeof token !== "string" || !token))
    throw new Error("Drive returned an invalid continuation token.");
  return { files, nextPageToken: typeof token === "string" ? token : null };
}
export interface ChangesPage {
  changes: { fileId: string; removed: boolean; file: DriveFile | null }[];
  nextPageToken: string | null;
  newStartPageToken: string | null;
}
export function parseChangesPage(value: unknown): ChangesPage {
  if (typeof value !== "object" || value === null)
    throw new Error("Drive returned an invalid change list.");
  const page = value as Record<string, unknown>;
  if (!Array.isArray(page.changes))
    throw new Error("Drive returned an invalid change list.");
  const next = page.nextPageToken;
  const start = page.newStartPageToken;
  if (
    (typeof next !== "string" && typeof start !== "string") ||
    (typeof next === "string" && !next) ||
    (typeof start === "string" && !start)
  )
    throw new Error(
      "Drive returned an incomplete change list. Retry discovery before trusting these profiles.",
    );
  return {
    changes: page.changes.map((raw) => {
      const change = raw as Record<string, unknown>;
      if (typeof change.fileId !== "string")
        throw new Error("Drive returned an invalid change entry.");
      return {
        fileId: change.fileId,
        removed: change.removed === true,
        file: change.removed === true ? null : parseDriveFile(change.file),
      };
    }),
    nextPageToken: typeof next === "string" ? next : null,
    newStartPageToken: typeof start === "string" ? start : null,
  };
}
export function parseStartPageToken(value: unknown): string {
  const token =
    typeof value === "object" && value !== null && "startPageToken" in value
      ? value.startPageToken
      : null;
  if (typeof token !== "string" || !token)
    throw new Error("Drive did not provide a change token.");
  return token;
}
export function parseMedia(value: unknown): string {
  const media =
    typeof value === "object" && value !== null && "media" in value
      ? value.media
      : null;
  if (typeof media !== "string")
    throw new Error("Drive did not return the profile file contents.");
  return media;
}
/// Metadata for a new immutable operation file with a reserved identity.
export function fileMetadata(
  id: string,
  namespaceDigest: string,
  operation: Operation,
  sha256: string,
) {
  return {
    id,
    name: fileName(operation.operation),
    mimeType: "application/json",
    parents: ["appDataFolder"],
    appProperties: {
      shepType: "profile",
      shepFormat: "operation-v1",
      shepNamespace: namespaceDigest,
      shepProfile: operation.profile,
      shepGeneration: operation.generation,
      shepOperation: operation.operation,
      shepSha256: sha256,
    },
  };
}
export function isMissing(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    "missing" in value &&
    value.missing === true
  );
}
