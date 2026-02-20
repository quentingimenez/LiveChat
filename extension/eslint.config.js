import tsEslintPlugin from '@typescript-eslint/eslint-plugin';
import tsEslintParser from '@typescript-eslint/parser';

const tsRules = tsEslintPlugin.configs.recommended.rules;

export default [
  {
    ignores: ['dist/**', 'node_modules/**', 'release/**'],
  },
  {
    files: ['**/*.ts', '**/*.tsx'],
    languageOptions: {
      parser: tsEslintParser,
      ecmaVersion: 'latest',
      sourceType: 'module',
    },
    plugins: {
      '@typescript-eslint': tsEslintPlugin,
    },
    rules: {
      ...tsRules,
      'no-undef': 'off',
      '@typescript-eslint/consistent-type-imports': [
        'error',
        {
          prefer: 'type-imports',
        },
      ],
    },
  },
];
