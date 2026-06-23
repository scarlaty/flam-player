/**
 * lv_conf.h — Configuration LVGL pour Flam Player
 * LVGL 8.3.x, rendu logiciel 320x240, 32-bit couleur
 */

#ifndef LV_CONF_H
#define LV_CONF_H

#include <stdint.h>

/* Couleur */
#define LV_COLOR_DEPTH          32
#define LV_COLOR_16_SWAP        0
#define LV_COLOR_SCREEN_TRANSP  0
#define LV_COLOR_MIX_ROUND_OFS  128
#define LV_COLOR_CHROMA_KEY     lv_color_hex(0x00ff00)

/* Mémoire */
#define LV_MEM_CUSTOM           0
#define LV_MEM_SIZE             (256U * 1024U)
#define LV_MEM_ADR              0
#define LV_MEM_BUF_MAX_NUM      16

/* HAL */
#define LV_DISP_DEF_REFR_PERIOD 16      /* ~60 FPS */
#define LV_INDEV_DEF_READ_PERIOD 30
#define LV_TICK_CUSTOM           1
#define LV_TICK_CUSTOM_INCLUDE   "SDL.h"
#define LV_TICK_CUSTOM_SYS_TIME_EXPR (SDL_GetTicks())
#define LV_DPI_DEF               130

/* Dessin */
#define LV_DRAW_COMPLEX          1
#define LV_SHADOW_CACHE_SIZE     0
#define LV_CIRCLE_CACHE_SIZE     4
#define LV_IMG_CACHE_DEF_SIZE    0
#define LV_GRADIENT_MAX_STOPS    2
#define LV_DISP_ROT_MAX_BUF     (10U * 1024U)

/* GPU */
#define LV_USE_GPU_STM32_DMA2D   0
#define LV_USE_GPU_NXP_PXP       0
#define LV_USE_GPU_NXP_VG_LITE   0
#define LV_USE_GPU_SDL           0

/* Logging */
#define LV_USE_LOG               1
#define LV_LOG_LEVEL             LV_LOG_LEVEL_WARN
#define LV_LOG_PRINTF            1

/* Asserts */
#define LV_USE_ASSERT_NULL       1
#define LV_USE_ASSERT_MALLOC     1
#define LV_USE_ASSERT_STYLE      0
#define LV_USE_ASSERT_MEM_INTEGRITY  0
#define LV_USE_ASSERT_OBJ        0

/* Fonctionnalités */
#define LV_USE_PERF_MONITOR      0
#define LV_USE_MEM_MONITOR       0
#define LV_USE_REFR_DEBUG        0
#define LV_USE_USER_DATA         1
#define LV_ATTRIBUTE_MEM_ALIGN_SIZE 1

/* Polices intégrées LVGL (on les désactive, on fournit les nôtres) */
#define LV_FONT_MONTSERRAT_8    0
#define LV_FONT_MONTSERRAT_10   0
#define LV_FONT_MONTSERRAT_12   0
#define LV_FONT_MONTSERRAT_14   1   /* On garde une police de fallback */
#define LV_FONT_MONTSERRAT_16   0
#define LV_FONT_MONTSERRAT_18   0
#define LV_FONT_MONTSERRAT_20   0
#define LV_FONT_MONTSERRAT_22   0
#define LV_FONT_MONTSERRAT_24   0
#define LV_FONT_MONTSERRAT_26   0
#define LV_FONT_MONTSERRAT_28   0
#define LV_FONT_MONTSERRAT_30   0
#define LV_FONT_MONTSERRAT_32   0
#define LV_FONT_MONTSERRAT_34   0
#define LV_FONT_MONTSERRAT_36   0
#define LV_FONT_MONTSERRAT_38   0
#define LV_FONT_MONTSERRAT_40   0
#define LV_FONT_MONTSERRAT_42   0
#define LV_FONT_MONTSERRAT_44   0
#define LV_FONT_MONTSERRAT_46   0
#define LV_FONT_MONTSERRAT_48   0

#define LV_FONT_MONTSERRAT_28_COMPRESSED  0
#define LV_FONT_DEJAVU_16_PERSIAN_HEBREW  0
#define LV_FONT_SIMSUN_16_CJK             0
#define LV_FONT_UNSCII_8                  0
#define LV_FONT_UNSCII_16                 0

#define LV_FONT_DEFAULT         &lv_font_montserrat_14
#define LV_FONT_CUSTOM_DECLARE

#define LV_FONT_FMT_TXT_LARGE   0
#define LV_USE_FONT_COMPRESSED   0
#define LV_USE_FONT_SUBPX        0
#define LV_FONT_SUBPX_BGR        0

/* Texte */
#define LV_TXT_ENC              LV_TXT_ENC_UTF8
#define LV_TXT_BREAK_CHARS      " ,.;:-_"
#define LV_TXT_LINE_BREAK_LONG_LEN   0
#define LV_TXT_LINE_BREAK_LONG_PRE_MIN_LEN   3
#define LV_TXT_LINE_BREAK_LONG_POST_MIN_LEN  3
#define LV_TXT_COLOR_CMD        "#"

/* Widgets */
#define LV_USE_ARC              1
#define LV_USE_BAR              1
#define LV_USE_BTN              1
#define LV_USE_BTNMATRIX        1
#define LV_USE_CANVAS           0
#define LV_USE_CHECKBOX         1
#define LV_USE_DROPDOWN         1
#define LV_USE_IMG              1
#define LV_USE_LABEL            1
#define LV_USE_LINE             1
#define LV_USE_ROLLER           1
#define LV_USE_SLIDER           1
#define LV_USE_SWITCH           1
#define LV_USE_TABLE            0
#define LV_USE_TEXTAREA         0

/* Widgets extra */
#define LV_USE_ANIMIMG          1
#define LV_USE_CALENDAR         0
#define LV_USE_CHART            0
#define LV_USE_COLORWHEEL       0
#define LV_USE_IMGBTN           1
#define LV_USE_KEYBOARD         0
#define LV_USE_LED              0
#define LV_USE_LIST             0
#define LV_USE_MENU             0
#define LV_USE_METER            0
#define LV_USE_MSGBOX           0
#define LV_USE_SPAN             0
#define LV_USE_SPINBOX          0
#define LV_USE_SPINNER          1
#define LV_USE_TABVIEW          0
#define LV_USE_TILEVIEW         0
#define LV_USE_WIN              0

/* Layouts */
#define LV_USE_FLEX             1
#define LV_USE_GRID             0

/* Thèmes */
#define LV_USE_THEME_DEFAULT    0
#define LV_USE_THEME_MONO       0
#define LV_USE_THEME_BASIC      0

/* Décodeur d'image intégré */
#define LV_USE_PNG              0
#define LV_USE_BMP              0
#define LV_USE_SJPG             0
#define LV_USE_GIF              0
#define LV_USE_QRCODE           0
#define LV_USE_FREETYPE         0
#define LV_USE_RLOTTIE          0
#define LV_USE_FFMPEG           0

/* Fichier système */
#define LV_USE_FS_STDIO         0
#define LV_USE_FS_POSIX         0
#define LV_USE_FS_WIN32         0
#define LV_USE_FS_FATFS         0

/* Snapshot */
#define LV_USE_SNAPSHOT          0

#endif /* LV_CONF_H */
