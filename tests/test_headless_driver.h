#ifndef TEST_HEADLESS_DRIVER_H
#define TEST_HEADLESS_DRIVER_H

#include "lvgl/lvgl.h"

int  test_driver_init(void);
void test_driver_tick(void);
void test_driver_quit(void);

lv_indev_t *test_driver_get_indev(void);

#endif
