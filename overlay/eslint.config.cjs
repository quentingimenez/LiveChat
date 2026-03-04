const js = require('@eslint/js');
const globals = require('globals');
const tseslint = require('typescript-eslint');

module.exports = [
  {
    ignores: ['dist/**', 'node_modules/**', 'eslint.config.cjs']
  },
  js.configs.recommended,
  {
    files: ['src/**/*.ts', 'src/**/*.d.ts'],
    languageOptions: {
      parser: tseslint.parser,
      parserOptions: {
        project: false
      },
      globals: {
        ...globals.node,
        ...globals.browser
      }
    },
    plugins: {
      '@typescript-eslint': tseslint.plugin
    },
    rules: {
      ...tseslint.configs.recommended.rules,
      'no-console': 'off',
      'no-unused-vars': 'off',
      '@typescript-eslint/consistent-type-imports': ['error', { prefer: 'type-imports' }],
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }]
    }
  },
  {
    files: ['src/**/*.d.ts'],
    rules: {
      '@typescript-eslint/no-unused-vars': 'off'
    }
  },
  {
    files: ['main.js', 'main.runtime.js', 'preload.js', 'protocol.js', 'renderer/**/*.js'],
    languageOptions: {
      globals: {
        ...globals.node,
        ...globals.browser
      }
    },
    rules: {
      'no-console': 'off',
      'no-unused-vars': 'off'
    }
  },
  {
    files: ['tests/**/*.js'],
    languageOptions: {
      globals: {
        ...globals.node
      }
    },
    rules: {
      'no-console': 'off',
      'no-unused-vars': 'off'
    }
  }
];
