#include "PierCoreBridge.h"

#include "pier_core.h"

PierCoreBridge::PierCoreBridge(QObject *parent)
    : QObject(parent)
{
}

QString PierCoreBridge::version() const
{
    // pier_core_version() returns a static UTF-8 C string owned by the Rust
    // crate; we copy into a QString so the QML side can treat it as a
    // normal value type.
    return QString::fromUtf8(pier_core_version());
}

QString PierCoreBridge::buildInfo() const
{
    return QString::fromUtf8(pier_core_build_info());
}

QString PierCoreBridge::qtVersion() const
{
    // qVersion() is the runtime Qt version the app was linked against,
    // which is what the status bar should show (vs QT_VERSION_STR which
    // is baked in at compile time of this translation unit).
    return QString::fromUtf8(qVersion());
}

bool PierCoreBridge::hasFeature(const QString &name) const
{
    const QByteArray utf8 = name.toUtf8();
    return pier_core_has_feature(utf8.constData()) != 0;
}
