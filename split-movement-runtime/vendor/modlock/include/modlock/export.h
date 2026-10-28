#pragma once

#if defined(_WIN32)
#if defined(MODLOCK_BUILDING_SDK)
#define MODLOCK_API __declspec(dllexport)
#else
#define MODLOCK_API __declspec(dllimport)
#endif
#else
#define MODLOCK_API __attribute__((visibility("default")))
#endif
