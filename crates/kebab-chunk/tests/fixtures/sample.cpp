#include <string>
#include <vector>

namespace kebab {
namespace chunk {

class MdHeadingV1Chunker {
public:
    MdHeadingV1Chunker() = default;
    ~MdHeadingV1Chunker() = default;

    std::string chunk_doc(const std::string& doc) {
        return doc;
    }

    int operator()(int x) const {
        return x * 2;
    }

private:
    int counter_ = 0;
};

template <typename T>
T identity(T value) {
    return value;
}

}  // namespace chunk

void global_helper() {
    // free function in kebab namespace
}

}  // namespace kebab

int main() {
    kebab::chunk::MdHeadingV1Chunker c;
    return 0;
}
