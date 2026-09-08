# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# EXAMPLE CUSTOM SEGMENT - Docker Status
# ============================================================================

function segment_docker_detect
    __has_binary docker; and test -f docker-compose.yml -o -f Dockerfile
end

function segment_docker_render --argument-names is_last
    if docker info >/dev/null 2>&1
        set -l containers (docker ps -q | wc -l | string trim)
        if test $containers -gt 0
            gpy_section_start blue white "🐳 $containers"
            gpy_section_end blue
        else
            gpy_section_start cyan white "🐳"
            gpy_section_end cyan
        end
    else
        gpy_section_start red white "🐳 ✗"
        gpy_section_end red
    end
end
