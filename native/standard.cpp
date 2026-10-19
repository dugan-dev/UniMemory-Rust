#include <cstddef>
#include <new>

extern "C" void* unimemory_standard_allocate(std::size_t bytes, std::size_t alignment) noexcept {
    if (alignment > __STDCPP_DEFAULT_NEW_ALIGNMENT__) {
        return ::operator new(bytes, std::align_val_t(alignment), std::nothrow);
    }
    return ::operator new(bytes, std::nothrow);
}

extern "C" void unimemory_standard_deallocate(void* pointer, std::size_t alignment) noexcept {
    if (alignment > __STDCPP_DEFAULT_NEW_ALIGNMENT__) {
        ::operator delete(pointer, std::align_val_t(alignment));
    } else {
        ::operator delete(pointer);
    }
}
