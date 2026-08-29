export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className !== undefined && className !== "") node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export function append(parent: ParentNode, ...kids: Array<Node | string | null | undefined>): void {
  for (const kid of kids) {
    if (kid === null || kid === undefined) continue;
    parent.append(kid);
  }
}

export function clear(node: Element): void {
  node.replaceChildren();
}

export function setClass(node: Element, token: string, on: boolean): void {
  node.classList.toggle(token, on);
}
