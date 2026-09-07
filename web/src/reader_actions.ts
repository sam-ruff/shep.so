/** The surrounding view still uses generated DOM. Keep the same message's
 * action branch connected throughout replacement: even momentary removal
 * cancels a native press between pointer-down and pointer-up. Only these static
 * structural ancestors and four action buttons are retained. Other controls
 * keep their normal construction, listeners and captured local state. */
export function renderReaderTree(root: HTMLElement, next: HTMLElement) {
  const oldActions = root.querySelector<HTMLElement>(".reader-actions"),
    newActions = next.querySelector<HTMLElement>(".reader-actions");
  const replace = () => root.replaceChildren(...next.childNodes);
  if (
    !oldActions ||
    !newActions ||
    oldActions.dataset.message !== newActions.dataset.message ||
    oldActions.children.length !== newActions.children.length ||
    [...oldActions.children].some(
      (node, i) => node.tagName !== newActions.children[i].tagName,
    )
  ) {
    replace();
    return;
  }
  const path = (leaf: HTMLElement, ancestor: HTMLElement) => {
    const result: HTMLElement[] = [];
    for (let node: HTMLElement | null = leaf; node; node = node.parentElement) {
      result.unshift(node);
      if (node === ancestor) return result;
    }
    return [];
  };
  const before = path(oldActions, root),
    after = path(newActions, next);
  if (
    before.length !== after.length ||
    before.some((node, i) => node.tagName !== after[i].tagName)
  ) {
    replace();
    return;
  }
  const attributes = (old: Element, fresh: Element) => {
    for (const a of [...old.attributes])
      if (!fresh.hasAttribute(a.name)) old.removeAttribute(a.name);
    for (const a of fresh.attributes)
      if (old.getAttribute(a.name) !== a.value)
        old.setAttribute(a.name, a.value);
  };
  for (let depth = 0; depth < before.length - 1; depth++) {
    const old = before[depth],
      fresh = after[depth],
      branch = before[depth + 1],
      replacement = after[depth + 1];
    if (depth) attributes(old, fresh); // the app root owns its layout variables
    // Never move the retained branch, including when its siblings change.
    // Inserting it again would reset native activation just like removal.
    for (const child of [...old.childNodes])
      if (child !== branch) child.remove();
    let following = false;
    for (const child of [...fresh.childNodes]) {
      if (child === replacement) following = true;
      else if (following) old.append(child);
      else old.insertBefore(child, branch);
    }
  }
  attributes(oldActions, newActions);
  for (let i = 0; i < newActions.children.length; i++) {
    const old = oldActions.children[i] as HTMLButtonElement,
      fresh = newActions.children[i] as HTMLButtonElement;
    attributes(old, fresh);
    old.onclick = fresh.onclick;
    old.disabled = fresh.disabled;
    old.querySelector("span")!.textContent =
      fresh.querySelector("span")!.textContent;
    old
      .querySelector("path")!
      .setAttribute("d", fresh.querySelector("path")!.getAttribute("d")!);
  }
}
