export function nextIndexForKey(
  key: string,
  currentIndex: number,
  itemCount: number,
): number | null {
  switch (key) {
    case "ArrowDown":
    case "ArrowRight":
      return clampIndex(currentIndex + 1, itemCount);
    case "ArrowUp":
    case "ArrowLeft":
      return clampIndex(currentIndex - 1, itemCount);
    case "Home":
      return clampIndex(0, itemCount);
    case "End":
      return clampIndex(itemCount - 1, itemCount);
    default:
      return null;
  }
}

export function clampIndex(index: number, itemCount: number) {
  if (itemCount <= 0) {
    return 0;
  }

  return Math.min(Math.max(index, 0), itemCount - 1);
}

export function isActivationKey(key: string) {
  return key === "Enter" || key === " ";
}
