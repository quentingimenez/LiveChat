import { fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { describe, expect, it, vi } from 'vitest';
import { LoginScreen } from './LoginScreen';

describe('LoginScreen', () => {
  it('renders both login methods and the OAuth error message', () => {
    render(
      <LoginScreen
        methods={{ discord: true, secret: true }}
        errorCode="not_allowed"
        onSubmitSecret={vi.fn()}
      />,
    );

    const discord = screen.getByRole('link', { name: /Se connecter avec Discord/ });
    expect(discord.getAttribute('href')).toBe('/admin/auth/discord');
    expect(screen.getByLabelText(/Secret d’administration/)).toBeTruthy();
    expect(screen.getByText(/n’est pas autorisé/)).toBeTruthy();
    expect(screen.getByRole('button', { name: /Se connecter avec le secret/ })).toBeTruthy();
  });

  it('hides the Discord link when only the secret method is available', () => {
    render(
      <LoginScreen methods={{ discord: false, secret: true }} onSubmitSecret={vi.fn()} />,
    );

    expect(screen.queryByRole('link', { name: /Se connecter avec Discord/ })).toBeNull();
    expect(screen.getByLabelText(/Secret d’administration/)).toBeTruthy();
  });

  it('shows a readable state when no method is available', () => {
    render(
      <LoginScreen methods={{ discord: false, secret: false }} onSubmitSecret={vi.fn()} />,
    );

    expect(screen.getByText(/Aucune méthode de connexion/)).toBeTruthy();
  });

  it('submits the secret through the callback', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<LoginScreen methods={{ discord: false, secret: true }} onSubmitSecret={onSubmit} />);

    fireEvent.input(screen.getByLabelText(/Secret d’administration/), {
      target: { value: 's3cret' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Se connecter avec le secret/ }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith('s3cret'));
  });
});
