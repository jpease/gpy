#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later

function migrate_from_existing_prompt
    echo "🔄 Migrating from existing Fish prompt configuration..."

    set -l config_file ~/.config/fish/config.fish
    set -l backup_file ~/.config/fish/config.fish.backup.(date +%Y%m%d_%H%M%S)

    if test -f $config_file
        cp $config_file $backup_file
        echo "📋 Backed up existing config to: $backup_file"
    end

    # Common prompt variables to preserve
    set -l preserve_vars \
        fish_color_command \
        fish_color_param \
        fish_color_error \
        fish_prompt_pwd_dir_length

    echo "💾 Preserving existing Fish color settings..."
    for var in $preserve_vars
        if set -q $var
            set -l val $$var
            echo "  $var = $val"
        end
    end

    echo "✅ Migration complete!"
    echo "💡 Your old config is backed up and existing colors are preserved."
end

function restore_backup_config
    echo "🔙 Restoring from backup..."

    set -l backups ~/.config/fish/config.fish.backup.*
    if test (count $backups) -eq 0
        echo "❌ No backup files found"
        return 1
    end

    # Get most recent backup
    set -l latest_backup (ls -t $backups | head -1)

    cp $latest_backup ~/.config/fish/config.fish
    echo "✅ Restored config from: $latest_backup"
    echo "🔄 Restart your shell to apply changes"
end

if test (count $argv) -eq 0
    migrate_from_existing_prompt
else
    switch $argv[1]
        case backup
            migrate_from_existing_prompt
        case restore
            restore_backup_config
        case '*'
            echo "Usage: fish migrate.fish [backup|restore]"
    end
end
