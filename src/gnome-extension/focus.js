export function extractFocus(window) {
  let windowClass = '';
  let windowTitle = '';

  if (window) {
    const classValue = window.get_wm_class();
    const titleValue = window.get_title();
    if (classValue) {
      windowClass = classValue;
    }
    if (titleValue) {
      windowTitle = titleValue;
    }
  }

  return { windowClass, windowTitle };
}
