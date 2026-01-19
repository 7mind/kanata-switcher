const EMPTY_LAYER = '';
const SOURCE_EXTERNAL = 'external';
const SOURCE_FOCUS = 'focus';

export function isDaemonOwnerAvailable(owner) {
  if (owner === null || owner === undefined) {
    return false;
  }
  if (typeof owner !== 'string') {
    throw new Error(`Daemon owner must be string, got ${typeof owner}`);
  }
  if (owner.trim().length === 0) {
    return false;
  }
  return true;
}

export function disconnectedState() {
  const status = {
    layer: EMPTY_LAYER,
    virtualKeys: [],
    source: SOURCE_EXTERNAL
  };
  const focusStatus = {
    layer: EMPTY_LAYER,
    virtualKeys: [],
    source: SOURCE_FOCUS
  };
  return {
    status,
    focusStatus,
    lastStatus: status,
    paused: false
  };
}
