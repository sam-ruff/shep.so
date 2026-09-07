import { mailMetadata, type CacheMail } from "./cache_changes";
import type { Change } from "./storage";
import type { BulkIdentity } from "./bulk_journal";
import type { MailAlias } from "./sent_cache";

export type PhysicalIdentity = Pick<
  BulkIdentity,
  "id" | "account" | "folder" | "remoteId"
>;
/** Attached only after a provider receipt, validated recovery, or an atomic
 * local action. It is transaction input, never a speculative server identity. */
export interface IdentityTransition {
  before: PhysicalIdentity;
  after: PhysicalIdentity;
}
export const samePhysical = (a: PhysicalIdentity, b: PhysicalIdentity) =>
  a.id === b.id &&
  a.account === b.account &&
  a.folder === b.folder &&
  a.remoteId === b.remoteId;
export const metadataIdentity = (m: CacheMail): PhysicalIdentity => ({
  id: m.id,
  account: m.core.account_id,
  folder: m.core.folder,
  remoteId: m.moved ? "" : m.core.remote_id,
});
export function sameReviewedSource(
  expected: BulkIdentity,
  actual: BulkIdentity,
) {
  if (expected.account !== actual.account) return false;
  if (expected.lineage)
    return (
      expected.lineage === actual.lineage &&
      (expected.anchor ?? expected.id) === (actual.anchor ?? actual.id)
    );
  return samePhysical(expected, actual);
}
const read = <T>(r: IDBRequest<T>) =>
  new Promise<T>((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });

/** Read only metadata before any writes, in the same cache transaction. Trusted
 * moves retain origin; an unproven physical replacement gets a new origin.
 * Alias continuity is retained only through the verified canonical merge. */
export async function prepareLineage(tx: IDBTransaction, changes: Change[]) {
  const metadata = tx.objectStore("mailMetadata"),
    aliases = tx.objectStore("mailAliases"),
    mail = changes.filter((c) => c.store === "mail"),
    links = changes.filter((c) => c.store === "mailAliases" && c.value),
    previousAliases = new Map<string, MailAlias>();
  await Promise.all(
    links.map(async (c) => {
      const previous = await read<MailAlias | undefined>(aliases.get(c.key));
      if (previous) previousAliases.set(c.key, previous);
    }),
  );
  const ids = new Set([
    ...mail.map((c) => c.key),
    ...links.flatMap((c) => [c.key, (c.value as MailAlias).target]),
    ...[...previousAliases.values()].map((a) => a.target),
  ]);
  const old = new Map<string, CacheMail | undefined>();
  await Promise.all(
    [...ids].map(async (id) =>
      old.set(id, await read<CacheMail | undefined>(metadata.get(id))),
    ),
  );
  const updated = new Map<string, CacheMail | undefined>();
  for (const c of mail) {
    const value = mailMetadata(c.value),
      previous = old.get(c.key);
    if (value && previous) {
      const before = metadataIdentity(previous),
        after = metadataIdentity(value);
      if (
        samePhysical(before, after) ||
        (c.identity &&
          samePhysical(before, c.identity.before) &&
          samePhysical(after, c.identity.after))
      )
        value.lineage = previous.lineage ?? value.lineage;
    }
    updated.set(c.key, value);
  }
  for (const c of links) {
    const alias = c.value as MailAlias,
      previous = previousAliases.get(c.key),
      source = old.get(c.key),
      target = updated.has(alias.target)
        ? updated.get(alias.target)
        : old.get(alias.target);
    // Explicit copies prevent caller-owned objects changing if the transaction aborts.
    const next: MailAlias = { alias: alias.alias, target: alias.target };
    if (target?.lineage) {
      if (
        source?.lineage &&
        source.core.account_id === target.core.account_id
      ) {
        next.lineage = source.lineage;
        next.targetLineage = target.lineage;
      } else if (previous?.lineage && previous.targetLineage) {
        const former = old.get(previous.target);
        const continued =
          previous.target === alias.target ||
          links.some(
            (link) =>
              link.key === previous.target &&
              (link.value as MailAlias).target === alias.target,
          );
        if (
          continued &&
          former?.lineage === previous.targetLineage &&
          former.core.account_id === target.core.account_id
        ) {
          next.lineage = previous.lineage;
          next.targetLineage = target.lineage;
        }
      }
    }
    c.value = next;
  }
  return updated;
}
