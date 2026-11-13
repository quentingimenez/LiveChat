export const inpageStyles = `
.lce-root {
  all: initial;
}

.lce-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-height: 28px;
  border: 1px solid transparent;
  border-radius: 999px;
  padding: 4px 12px;
  font-family: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  font-size: 12px;
  font-weight: 600;
  line-height: 1;
  letter-spacing: 0.02em;
  cursor: pointer;
  user-select: none;
  /* LiveChat charter (docs/brand/README.md): amber means "goes on screen". */
  background: #f2b544;
  color: #2a1d02;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
  transition: background-color 120ms ease, opacity 120ms ease;
}

.lce-button:hover {
  background: #f7c466;
}

.lce-button:focus-visible {
  outline: 2px solid #7fa8f5;
  outline-offset: 2px;
}

.lce-button[disabled] {
  opacity: 0.6;
  cursor: wait;
}

.lce-button-floating {
  position: fixed;
  right: 20px;
  bottom: 24px;
  z-index: 2147483646;
}

.lce-toast-container {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 2147483647;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.lce-toast {
  min-width: 200px;
  max-width: 360px;
  border-radius: 10px;
  padding: 10px 12px;
  font-family: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  font-size: 13px;
  font-weight: 600;
  color: #e9ebf3;
  background: #262c3c;
  border: 1px solid #3a4259;
  border-left: 3px solid #7fa8f5;
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.35);
  opacity: 0;
  transform: translateY(8px);
  animation: lce-toast-in 180ms ease forwards;
}

.lce-toast-success {
  border-left-color: #5fc9a3;
}

.lce-toast-error {
  border-left-color: #ec6a70;
}

.lce-toast-info {
  border-left-color: #7fa8f5;
}

@keyframes lce-toast-in {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
`;
