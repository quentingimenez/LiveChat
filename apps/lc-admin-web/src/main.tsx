import { render } from 'preact';
import { App } from './app';
import './styles/base.css';
import './styles/app.css';

const root = document.getElementById('app');
if (root) {
  render(<App />, root);
}

// The mockup used a hash link for the skip link; in a hash-routed app that
// would navigate. Focus the main region directly instead.
const skip = document.querySelector<HTMLAnchorElement>('a[data-skip]');
skip?.addEventListener('click', (event) => {
  event.preventDefault();
  document.getElementById('content')?.focus();
});
