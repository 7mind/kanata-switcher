const MAX_VK_COUNT_DIGIT = 9;
const MIN_MULTI_VK_COUNT = 2;
const INFINITY_SYMBOL = '\u221e';

export function formatLayerLetter(layerName) {
  if (typeof layerName !== 'string') {
    return '?';
  }

  const trimmed = layerName.trim();
  if (!trimmed) {
    return '?';
  }

  return trimmed[0].toUpperCase();
}

export function formatVirtualKeys(virtualKeys) {
  if (!Array.isArray(virtualKeys)) {
    return '';
  }

  const count = virtualKeys.length;
  if (count === 0) {
    return '';
  }

  if (count === 1) {
    const name = virtualKeys[0];
    if (typeof name !== 'string') {
      return '';
    }
    const trimmed = name.trim();
    if (!trimmed) {
      return '';
    }
    return trimmed[0].toUpperCase();
  }

  if (count < MIN_MULTI_VK_COUNT) {
    return '';
  }

  if (count > MAX_VK_COUNT_DIGIT) {
    return INFINITY_SYMBOL;
  }

  return String(count);
}

export function selectStatus(showFocusOnly, focusStatus, lastStatus) {
  return showFocusOnly ? focusStatus : lastStatus;
}
