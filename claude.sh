#!/bin/bash

export ANTHROPIC_AUTH_TOKEN=sk-15c6f97850c8440c82cc657208cb550f
export ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic

claude --model deepseek-v4-pro  --allow-dangerously-skip-permissions
