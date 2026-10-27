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
  border: 1px solid rgba(255, 255, 255, 0.25);
  border-radius: 999px;
  padding: 4px 12px;
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  font-size: 12px;
  font-weight: 700;
  line-height: 1;
  letter-spacing: 0.02em;
  cursor: pointer;
  user-select: none;
  background: linear-gradient(135deg, #ffca28 0%, #ff7043 100%);
  color: #111;
  box-shadow: 0 6px 14px rgba(255, 112, 67, 0.22);
  transition: transform 120ms ease, box-shadow 120ms ease, opacity 120ms ease;
}

.lce-button:hover {
  transform: translateY(-1px);
  box-shadow: 0 8px 16px rgba(255, 112, 67, 0.28);
}

.lce-button:active {
  transform: translateY(0);
  box-shadow: 0 4px 8px rgba(255, 112, 67, 0.24);
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
  font-family: "Segoe UI", "Helvetica Neue", sans-serif;
  font-size: 13px;
  font-weight: 600;
  color: #fff;
  box-shadow: 0 12px 24px rgba(0, 0, 0, 0.3);
  opacity: 0;
  transform: translateY(8px);
  animation: lce-toast-in 180ms ease forwards;
}

.lce-toast-success {
  background: linear-gradient(135deg, #2e7d32, #43a047);
}

.lce-toast-error {
  background: linear-gradient(135deg, #b71c1c, #e53935);
}

.lce-toast-info {
  background: linear-gradient(135deg, #1565c0, #1e88e5);
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
